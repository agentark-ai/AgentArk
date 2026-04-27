import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import { Alert, Box, Button, Chip, Divider, Stack, TextField, Typography } from "@mui/material";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../api/client";
import { PRODUCT_CATEGORY } from "../brand";
import type { BrowserHandoffStatus } from "../types";

type BrowserHandoffPageProps = {
  sessionId: string;
  onBack: () => void;
};

function isLocalHost(hostname: string): boolean {
  const normalized = (hostname || "").trim().toLowerCase();
  return normalized === "localhost" || normalized === "127.0.0.1" || normalized === "::1" || normalized === "[::1]";
}

function buildLiveViewUrl(status?: BrowserHandoffStatus | null): string | null {
  if (!status?.live_view_enabled || !status.live_view_port || !status.live_view_path) return null;
  if (typeof window === "undefined") return null;
  const hostname = window.location.hostname || "localhost";
  const path = status.live_view_path.startsWith("/") ? status.live_view_path : `/${status.live_view_path}`;
  return `http://${hostname}:${status.live_view_port}${path}`;
}

function statusTone(status: string): "success" | "warning" | "error" | "info" | "default" {
  switch ((status || "").toLowerCase()) {
    case "ready":
      return "success";
    case "operator_claimed":
      return "info";
    case "waiting_for_operator":
    case "awaiting_resume":
      return "warning";
    case "completed":
      return "success";
    case "failed":
    case "interrupted":
      return "error";
    default:
      return "default";
  }
}

export function BrowserHandoffPage({ sessionId, onBack }: BrowserHandoffPageProps) {
  const [note, setNote] = useState("");
  const statusQ = useQuery({
    queryKey: ["browser-handoff", sessionId],
    queryFn: async () =>
      (await api.rawGet(`/browser/sessions/${encodeURIComponent(sessionId)}`)) as BrowserHandoffStatus,
    refetchInterval: 2000,
    refetchIntervalInBackground: true,
  });

  const claimMutation = useMutation({
    mutationFn: async () =>
      (await api.rawPost(`/browser/sessions/${encodeURIComponent(sessionId)}/claim`)) as BrowserHandoffStatus,
    onSuccess: () => statusQ.refetch(),
  });
  const releaseMutation = useMutation({
    mutationFn: async () =>
      (await api.rawPost(`/browser/sessions/${encodeURIComponent(sessionId)}/release`)) as BrowserHandoffStatus,
    onSuccess: () => statusQ.refetch(),
  });
  const completeMutation = useMutation({
    mutationFn: async () =>
      (await api.rawPost(`/browser/sessions/${encodeURIComponent(sessionId)}/complete`, { note })) as BrowserHandoffStatus,
    onSuccess: () => statusQ.refetch(),
  });

  const status = statusQ.data;
  const liveUrl = useMemo(() => buildLiveViewUrl(status), [status]);
  const operatorHasControl = Boolean(status && (status.can_release || status.can_complete));
  const handoffErrorMessage = useMemo(() => {
    const raw = String((statusQ.error as Error)?.message || "Could not load browser handoff state.").trim();
    if (/\b404\b|not found/i.test(raw)) {
      return "Browser handoff session was not found. Open the full handoff link from chat instead of a shortened session id.";
    }
    return raw;
  }, [statusQ.error]);
  const remoteMixedContentRisk =
    typeof window !== "undefined" &&
    window.location.protocol === "https:" &&
    !isLocalHost(window.location.hostname || "");
  const liveBrowserLocked = Boolean(liveUrl && !remoteMixedContentRisk && !operatorHasControl);
  const liveViewLockMessage =
    status?.can_claim
      ? "Claim live browser to take control. Until then this session stays read-only."
      : "AgentArk currently holds this browser. This session stays read-only until control is handed back to you.";

  return (
    <Box
      sx={{
        minHeight: "100vh",
        color: "text.primary",
        px: { xs: 2, md: 3 },
        py: 2.5,
        background:
          "radial-gradient(circle at 12% 18%, var(--ui-rgba-255-255-255-050), transparent 34%), radial-gradient(circle at 84% 76%, var(--ui-rgba-158-184-255-060), transparent 28%), linear-gradient(180deg, #111216 0%, #0d0e11 52%, #0a0b0e 100%)",
      }}
    >
      <Stack spacing={2} sx={{ maxWidth: 1480, mx: "auto" }}>
        <Box className="glass-appbar" sx={{ px: 1.25, py: 1 }}>
          <Stack
            direction={{ xs: "column", lg: "row" }}
            sx={{ alignItems: { xs: "stretch", lg: "center" }, justifyContent: "space-between", gap: 1.25 }}
          >
            <Stack direction="row" spacing={1} sx={{ alignItems: "center", flexWrap: "wrap" }}>
              <Button startIcon={<ArrowBackRoundedIcon />} variant="outlined" onClick={onBack}>
                Back to chat
              </Button>
              <Box className="shell-brand-mark">
                <img src="/logo.svg" alt="AgentArk" width={36} height={36} />
              </Box>
              <Stack spacing={0.1} sx={{ minWidth: 0 }}>
                <Typography className="shell-kicker">AgentArk</Typography>
                <Typography className="shell-title">{PRODUCT_CATEGORY}</Typography>
              </Stack>
            </Stack>
            <Stack
              direction="row"
              spacing={1}
              sx={{ alignItems: "center", justifyContent: { xs: "space-between", lg: "flex-end" }, flexWrap: "wrap" }}
            >
              <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
                Browser handoff
              </Typography>
              {status ? <Chip size="small" color={statusTone(status.status)} label={status.status.replace(/_/g, " ")} /> : null}
              <Button startIcon={<RefreshRoundedIcon />} variant="text" onClick={() => statusQ.refetch()}>
                Refresh
              </Button>
            </Stack>
          </Stack>
        </Box>

        {statusQ.error ? <Alert severity="error">{handoffErrorMessage}</Alert> : null}
        {!status && !statusQ.error ? (
          <Alert severity="info">
            {statusQ.isLoading || statusQ.isFetching
              ? "Loading browser handoff session..."
              : "Waiting for browser handoff session details..."}
          </Alert>
        ) : null}
        {status ? (
          <Stack spacing={1.25}>
            <Box
              sx={{
                border: "1px solid var(--surface-border)",
                borderRadius: 2,
                p: 2,
                background: "var(--surface-bg-elevated)",
                backdropFilter: "blur(14px)",
                boxShadow: "var(--surface-shadow-soft)",
              }}
            >
              <Stack spacing={1}>
                <Typography variant="overline" sx={{ color: "text.secondary" }}>
                  Task
                </Typography>
                <Typography variant="h6" sx={{ fontWeight: 650, letterSpacing: 0 }}>
                  {status.task_description}
                </Typography>
                {status.question ? <Typography variant="body1">{status.question}</Typography> : null}
                {(status.page_title || status.page_url) ? (
                  <Typography variant="body2" sx={{ color: "text.secondary" }}>
                    {status.page_title || "Untitled page"} {status.page_url ? `| ${status.page_url}` : ""}
                  </Typography>
                ) : null}
                {status.summary ? <Alert severity="success">{status.summary}</Alert> : null}
                {status.reason ? (
                  <Alert severity={status.status === "interrupted" ? "warning" : "error"}>{status.reason}</Alert>
                ) : null}
              </Stack>
            </Box>

            <Stack direction={{ xs: "column", md: "row" }} spacing={1.25}>
              <Button variant="contained" disabled={!status.can_claim || claimMutation.isPending} onClick={() => claimMutation.mutate()}>
                {claimMutation.isPending ? "Claiming..." : "Claim live browser"}
              </Button>
              <Button variant="outlined" disabled={!status.can_release || releaseMutation.isPending} onClick={() => releaseMutation.mutate()}>
                {releaseMutation.isPending ? "Releasing..." : "Release hold"}
              </Button>
              <Button variant="contained" color="success" disabled={!status.can_complete || completeMutation.isPending} onClick={() => completeMutation.mutate()}>
                {completeMutation.isPending ? "Handing back..." : "Handoff back to AgentArk"}
              </Button>
              {liveUrl ? (
                operatorHasControl ? (
                  <Button variant="text" endIcon={<OpenInNewRoundedIcon />} href={liveUrl} target="_blank" rel="noreferrer">
                    Open live view
                  </Button>
                ) : (
                  <Button variant="text" endIcon={<OpenInNewRoundedIcon />} disabled>
                    Open live view
                  </Button>
                )
              ) : null}
            </Stack>

            <TextField
              label="What changed while you had control?"
              value={note}
              onChange={(event) => setNote(event.target.value)}
              minRows={3}
              multiline
              fullWidth
              size="small"
              helperText="Optional note for the agent before it resumes."
            />

            {remoteMixedContentRisk ? (
              <Alert severity="warning">This handoff page is open over HTTPS, but the live browser stream is local HTTP. Open AgentArk on the same machine running Docker to take over the browser directly.</Alert>
            ) : null}
            {liveBrowserLocked ? <Alert severity="info">{liveViewLockMessage}</Alert> : null}
            {!liveUrl && !remoteMixedContentRisk ? (
              <Alert severity="info">The live browser surface is not available yet. Keep this page open while AgentArk finishes wiring the handoff. The controls and live view refresh automatically.</Alert>
            ) : null}

            <Divider />

            {liveUrl && !remoteMixedContentRisk ? (
              <Box
                sx={{
                  position: "relative",
                  border: "1px solid var(--surface-border)",
                  borderRadius: 2,
                  overflow: "hidden",
                  minHeight: "70vh",
                  background: "var(--surface-bg)",
                  boxShadow: "var(--surface-shadow-soft)",
                }}
              >
                {liveBrowserLocked ? (
                  <Box
                    sx={{
                      position: "absolute",
                      inset: 0,
                      zIndex: 1,
                      display: "flex",
                      alignItems: "center",
                      justifyContent: "center",
                      p: 3,
                      bgcolor: "var(--ui-rgba-13-14-17-580)",
                    }}
                  >
                    <Box
                      sx={{
                        maxWidth: 460,
                        width: "100%",
                        border: "1px solid var(--button-border-strong)",
                        borderRadius: 2,
                        p: 2,
                        background: "var(--surface-bg-elevated)",
                        backdropFilter: "blur(14px)",
                        boxShadow: "var(--surface-shadow-soft)",
                      }}
                    >
                      <Stack spacing={1.25} sx={{ alignItems: "flex-start" }}>
                        <Typography variant="h6" sx={{ fontWeight: 650, letterSpacing: 0 }}>
                          Live browser is locked
                        </Typography>
                        <Typography variant="body2" sx={{ color: "text.secondary" }}>
                          {liveViewLockMessage}
                        </Typography>
                        {status?.can_claim ? (
                          <Button variant="contained" disabled={claimMutation.isPending} onClick={() => claimMutation.mutate()}>
                            {claimMutation.isPending ? "Claiming..." : "Claim live browser"}
                          </Button>
                        ) : null}
                      </Stack>
                    </Box>
                  </Box>
                ) : null}
                <iframe
                  title="Browser handoff live view"
                  src={liveUrl}
                  allow="clipboard-read; clipboard-write"
                  style={{
                    width: "100%",
                    height: "70vh",
                    border: 0,
                    display: "block",
                    background: "#141519",
                    pointerEvents: liveBrowserLocked ? "none" : "auto",
                  }}
                />
              </Box>
            ) : null}
          </Stack>
        ) : null}
      </Stack>
    </Box>
  );
}
