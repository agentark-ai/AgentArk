import ArrowBackRoundedIcon from "@mui/icons-material/ArrowBackRounded";
import OpenInNewRoundedIcon from "@mui/icons-material/OpenInNewRounded";
import RefreshRoundedIcon from "@mui/icons-material/RefreshRounded";
import { Alert, Box, Button, Chip, Divider, Stack, TextField, Typography } from "@mui/material";
import { useMutation, useQuery } from "@tanstack/react-query";
import { useMemo, useState } from "react";
import { api } from "../api/client";
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
  const remoteMixedContentRisk =
    typeof window !== "undefined" &&
    window.location.protocol === "https:" &&
    !isLocalHost(window.location.hostname || "");

  return (
    <Box sx={{ minHeight: "100vh", bgcolor: "#050816", color: "text.primary", px: { xs: 2, md: 3 }, py: 2.5 }}>
      <Stack spacing={2} sx={{ maxWidth: 1480, mx: "auto" }}>
        <Stack direction="row" sx={{ alignItems: "center", justifyContent: "space-between", gap: 1, flexWrap: "wrap" }}>
          <Stack direction="row" spacing={1} sx={{ alignItems: "center", flexWrap: "wrap" }}>
            <Button startIcon={<ArrowBackRoundedIcon />} variant="outlined" onClick={onBack}>
              Back to chat
            </Button>
            <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0 }}>
              Browser handoff
            </Typography>
            {status ? <Chip size="small" color={statusTone(status.status)} label={status.status.replace(/_/g, " ")} /> : null}
          </Stack>
          <Button startIcon={<RefreshRoundedIcon />} variant="text" onClick={() => statusQ.refetch()}>
            Refresh
          </Button>
        </Stack>

        {statusQ.error ? <Alert severity="error">{String((statusQ.error as Error)?.message || "Could not load browser handoff state.")}</Alert> : null}
        {status ? (
          <Stack spacing={1.25}>
            <Box sx={{ border: "1px solid rgba(120,145,182,0.18)", borderRadius: 2, p: 2, bgcolor: "rgba(8,16,30,0.88)" }}>
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
                <Button variant="text" endIcon={<OpenInNewRoundedIcon />} href={liveUrl} target="_blank" rel="noreferrer">
                  Open live view
                </Button>
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
            {!liveUrl && !remoteMixedContentRisk ? (
              <Alert severity="info">The live browser surface is not available yet. After the Docker rebuild, this page will embed the real browser session here.</Alert>
            ) : null}

            <Divider />

            {liveUrl && !remoteMixedContentRisk ? (
              <Box sx={{ border: "1px solid rgba(120,145,182,0.16)", borderRadius: 2, overflow: "hidden", minHeight: "70vh", bgcolor: "#02050d" }}>
                <iframe title="Browser handoff live view" src={liveUrl} style={{ width: "100%", height: "70vh", border: 0, display: "block", background: "#02050d" }} />
              </Box>
            ) : null}
          </Stack>
        ) : null}
      </Stack>
    </Box>
  );
}
