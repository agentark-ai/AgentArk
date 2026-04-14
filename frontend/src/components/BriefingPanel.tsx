import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Alert,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Stack,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import type { BriefingResponse, RecommendedAction } from "../types";
import { api } from "../api/client";
import { useCallback, useState } from "react";
import IconButton from "@mui/material/IconButton";
import CloseIcon from "@mui/icons-material/Close";

type Props = {
  briefing?: BriefingResponse;
  compact?: boolean;
};

export function BriefingPanel({ briefing, compact = false }: Props) {
  const queryClient = useQueryClient();
  const [execNotice, setExecNotice] = useState<{
    kind: "success" | "error" | "info";
    text: string;
  } | null>(null);
  const [dismissedActions, setDismissedActions] = useState<Set<string>>(new Set());
  const dismissAction = useCallback((id: string) => {
    setDismissedActions((prev) => new Set(prev).add(id));
  }, []);
  const actionableRisks = (briefing?.top_risks || []).filter((risk) => {
    const typedRisk = risk as Record<string, unknown>;
    const type = String(typedRisk.type || "").toLowerCase();
    const title = String(typedRisk.title || "").toLowerCase();
    return type !== "none" && title !== "no critical risks detected";
  });
  const visibleOpportunities = briefing?.top_opportunities || [];
  const visibleActions: RecommendedAction[] =
    briefing?.recommended_actions || briefing?.recommended_skills || [];
  const showSignalRow =
    actionableRisks.length > 0 || visibleOpportunities.length > 0;

  function asErrorMessage(err: unknown): string {
    if (!(err instanceof Error)) return "Request failed";
    const raw = err.message || "Request failed";
    try {
      const parsed = JSON.parse(raw) as { error?: string; message?: string };
      if (parsed.error && parsed.error.trim()) return parsed.error;
      if (parsed.message && parsed.message.trim()) return parsed.message;
    } catch {
      // ignore
    }
    return raw;
  }

  function summarizeExecResult(payload: unknown): string {
    const obj =
      payload && typeof payload === "object"
        ? (payload as Record<string, unknown>)
        : {};
    const result =
      obj.result && typeof obj.result === "object"
        ? (obj.result as Record<string, unknown>)
        : obj;
    const kind = String(result.kind || "");
    if (kind === "daily_brief_now")
      return "Daily Command Brief generated and pushed to your preferred channel.";
    if (kind === "create_task")
      return `Task queued: ${String(result.task_id || "") || "created"}.`;
    if (kind === "delegate") return "Delegation completed. Check Swarm for details.";
    if (String(result.status || "").includes("queued_for_approval"))
      return "Queued for approval. Review it in Tasks.";
    return "Executed.";
  }

  const executeAction = useMutation({
    mutationFn: api.executeRecommendedAction,
    onSuccess: async (out) => {
      setExecNotice({ kind: "success", text: summarizeExecResult(out) });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["status"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
    onError: (err) => {
      setExecNotice({ kind: "error", text: asErrorMessage(err) });
    },
  });

  if (!briefing) {
    return (
      <Card sx={compact ? { minHeight: 0 } : undefined}>
        <CardContent sx={compact ? { p: 1.25 } : undefined}>
          <Typography variant="h6">Daily Command Brief</Typography>
          <Typography
            sx={{
              color: "text.secondary",
              mt: 1
            }}>
            Waiting for briefing data...
          </Typography>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card sx={compact ? { minHeight: 0 } : undefined}>
      <CardContent
        sx={
          compact
            ? {
                p: 1.25,
                overflow: "auto",
              }
            : undefined
        }
      >
        <Stack
          direction="row"
          sx={{
            justifyContent: "space-between",
            alignItems: "center",
            mb: 1.5
          }}>
          <Typography variant="h6">Daily Command Brief</Typography>
          <Chip size="small" label={briefing.scope.toUpperCase()} />
        </Stack>
        {showSignalRow ? (
          <Grid2 container spacing={2}>
            {actionableRisks.length > 0 ? (
              <Grid2
                size={{ xs: 12, md: visibleOpportunities.length > 0 ? 6 : 12 }}
              >
                <Typography
                  variant="subtitle2"
                  sx={{
                    color: "warning.main",
                    mb: 1
                  }}>
                  Top Risks
                </Typography>
                <Stack spacing={1}>
                  {actionableRisks.slice(0, compact ? 2 : 3).map((risk, idx) => (
                    <Alert key={idx} severity="warning" variant="outlined">
                      {risk.title || "Risk"}:{" "}
                      {risk.summary || risk.detail || "No summary"}
                    </Alert>
                  ))}
                </Stack>
              </Grid2>
            ) : null}
            {visibleOpportunities.length > 0 ? (
              <Grid2
                size={{ xs: 12, md: actionableRisks.length > 0 ? 6 : 12 }}
              >
                <Typography
                  variant="subtitle2"
                  sx={{
                    color: "success.main",
                    mb: 1
                  }}>
                  Top Opportunities
                </Typography>
                <Stack spacing={1}>
                  {visibleOpportunities
                    .slice(0, compact ? 2 : 3)
                    .map((opp, idx) => (
                      <Alert key={idx} severity="success" variant="outlined">
                        {opp.title || "Opportunity"}:{" "}
                        {opp.summary || opp.detail || "No summary"}
                      </Alert>
                    ))}
                </Stack>
              </Grid2>
            ) : null}
          </Grid2>
        ) : null}

        {showSignalRow ? <Divider sx={{ my: 2 }} /> : null}

        {visibleActions.length > 0 ? (
          <Typography variant="subtitle2" sx={{
            mb: 1
          }}>
            Recommended Actions
          </Typography>
        ) : null}
        {execNotice ? <Alert severity={execNotice.kind}>{execNotice.text}</Alert> : null}
        <Stack spacing={1}>
          {visibleActions
            .filter((action) => !dismissedActions.has(action.id))
            .slice(0, compact ? 2 : 3)
            .map((action) => (
              <Stack
                key={action.id}
                direction={{ xs: "column", md: "row" }}
                spacing={1}
                className="action-row"
                sx={{
                  justifyContent: "space-between",
                  alignItems: { xs: "flex-start", md: "center" }
                }}>
                <Stack spacing={0.3} sx={{ flex: 1, minWidth: 0 }}>
                  <Typography variant="body2" sx={{
                    fontWeight: 700
                  }}>
                    {action.title}
                  </Typography>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    {action.summary || action.description || "No description"}
                  </Typography>
                </Stack>
                <Stack direction="row" spacing={0.5} sx={{
                  alignItems: "center"
                }}>
                  <Button
                    variant="contained"
                    size="small"
                    disabled={executeAction.isPending}
                    onClick={() => executeAction.mutate(action)}
                  >
                    {executeAction.isPending ? "Executing..." : "Execute"}
                  </Button>
                  <IconButton
                    size="small"
                    onClick={() => dismissAction(action.id)}
                    title="Dismiss"
                  >
                    <CloseIcon fontSize="small" />
                  </IconButton>
                </Stack>
              </Stack>
            ))}
        </Stack>
      </CardContent>
    </Card>
  );
}
