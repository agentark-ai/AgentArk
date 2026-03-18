import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Grid2,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import CloseIcon from "@mui/icons-material/Close";
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/client";
import { useUiStore } from "../store/uiStore";
import { AgentStatusBar } from "./AgentStatusBar";
import { WelcomeHero } from "./WelcomeHero";
import { NeedsAttentionInbox } from "./NeedsAttentionInbox";
import { TodaysHighlights } from "./TodaysHighlights";
import { SmartSuggestions } from "./SmartSuggestions";
import { ActivityFeed } from "./ActivityFeed";
import type { RecommendedSkill } from "../types";

const REFRESH_MS = 8000;
type PausePhase = "idle" | "stopping" | "stopped" | "resuming" | "resumed";
type AutomationObject = {
  id: string;
  kind: string;
  title: string;
  subtitle?: string | null;
  status: string;
  detail?: string | null;
  created_at?: string | null;
  next_run_at?: string | null;
  view: string;
  url?: string | null;
  enabled?: boolean | null;
  connected?: boolean | null;
};

type AutomationRun = {
  id: string;
  automation_id: string;
  kind: string;
  title: string;
  action: string;
  trigger: string;
  status: string;
  current_status?: string | null;
  attempt: number;
  started_at: string;
  completed_at?: string | null;
  duration_ms?: number | null;
  summary: string;
  output_preview?: string | null;
  error?: string | null;
  next_retry_at?: string | null;
  conversation_id?: string | null;
  project_id?: string | null;
  view: string;
};

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function pickAutomationObjects(raw: unknown): AutomationObject[] {
  const root = asRecord(raw);
  const items = root.objects;
  return Array.isArray(items) ? (items as AutomationObject[]) : [];
}

function pickAutomationRuns(raw: unknown): AutomationRun[] {
  const root = asRecord(raw);
  const items = root.runs;
  return Array.isArray(items) ? (items as AutomationRun[]) : [];
}

function automationKindLabel(kind: string): string {
  const normalized = (kind || "").toLowerCase();
  if (normalized === "task") return "Task";
  if (normalized === "watcher") return "Watcher";
  if (normalized === "app") return "App";
  if (normalized === "integration") return "Integration";
  return kind || "Automation";
}

function automationStatusColor(status: string): "success" | "warning" | "error" | "default" | "info" {
  const normalized = (status || "").toLowerCase();
  if (["running", "active", "connected", "completed", "triggered"].some((token) => normalized.includes(token))) {
    return "success";
  }
  if (["pending", "paused", "awaiting", "needs_auth", "not_configured"].some((token) => normalized.includes(token))) {
    return "warning";
  }
  if (["failed", "error", "cancelled", "stopped", "timed_out"].some((token) => normalized.includes(token))) {
    return "error";
  }
  if (normalized.includes("in_progress")) return "info";
  return "default";
}

function formatAutomationTime(value?: string | null): string {
  if (!value) return "-";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString();
}

function targetViewForAutomation(item: AutomationObject): string {
  if (item.view) return item.view;
  if (item.kind === "integration") return "settings";
  if (item.kind === "watcher") return "watchers";
  return item.kind;
}

function targetViewForAutomationRun(item: AutomationRun): string {
  if (item.view) return item.view;
  if (item.kind === "watcher") return "watchers";
  if (item.kind === "task") return "tasks";
  if (item.kind === "app") return "apps";
  return "trace";
}

function errMessage(error: unknown): string {
  if (error instanceof Error && error.message.trim()) return error.message;
  return "Request failed.";
}

type Props = {
  navigateToView: (view: string, replace?: boolean) => void;
  serverStatus?: { at: number; rtt_ms: number; status: import("../types").StatusResponse };
  serverError: boolean;
  serverLoading: boolean;
};

export function OverviewPane({ navigateToView, serverStatus, serverError, serverLoading }: Props) {
  const queryClient = useQueryClient();
  const autoRefresh = useUiStore((s) => s.autoRefresh);
  const interval = autoRefresh ? REFRESH_MS : false;
  const [pauseDialogOpen, setPauseDialogOpen] = useState(false);
  const [pausePhase, setPausePhase] = useState<PausePhase>("idle");
  const [pauseError, setPauseError] = useState<string | null>(null);
  const [pauseTarget, setPauseTarget] = useState<"pause" | "resume" | null>(null);
  const [inventoryOpen, setInventoryOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);

  // --- Data hooks ---
  const tasksQ = useQuery({ queryKey: ["tasks"], queryFn: api.getTasks, refetchInterval: interval });
  const traceQ = useQuery({ queryKey: ["trace"], queryFn: api.getTrace, refetchInterval: interval });
  const briefingQ = useQuery({ queryKey: ["briefing"], queryFn: api.getBriefing, refetchInterval: interval });
  const nudgesQ = useQuery({ queryKey: ["nudges"], queryFn: api.getNudges, refetchInterval: interval });
  const notificationsQ = useQuery({ queryKey: ["notifications"], queryFn: api.getNotifications, refetchInterval: interval });
  const securityQ = useQuery({
    queryKey: ["security-logs-dashboard"],
    queryFn: () => api.getSecurityLogs(5),
    refetchInterval: autoRefresh ? 30_000 : false,
  });
  const automationQ = useQuery({
    queryKey: ["automation-objects-dashboard"],
    queryFn: () => api.rawGet("/automation/objects"),
    refetchInterval: interval,
  });
  const automationRunsQ = useQuery({
    queryKey: ["automation-runs-dashboard"],
    queryFn: () => api.rawGet("/automation/runs"),
    refetchInterval: interval,
  });
  const settingsQ = useQuery({
    queryKey: ["settings-dashboard"],
    queryFn: api.getSettings,
    refetchInterval: false,
    staleTime: 60_000,
  });
  const autonomySettingsQ = useQuery({
    queryKey: ["autonomy-settings-dashboard"],
    queryFn: () => api.rawGet("/autonomy/settings"),
    refetchInterval: interval,
    staleTime: 10_000,
  });

  // --- Derived data ---
  const tasks = Array.isArray(tasksQ.data) ? tasksQ.data : [];
  const traces = traceQ.data?.history || [];
  const notifications = Array.isArray(notificationsQ.data) ? notificationsQ.data : [];
  const securityLogs = (securityQ.data as { logs?: Array<{ event_type: string; severity: string; message: string }> })?.logs || [];
  const nudges = nudgesQ.data?.nudges || [];
  const automationObjects = useMemo(() => pickAutomationObjects(automationQ.data), [automationQ.data]);
  const automationPreview = automationObjects.slice(0, 8);
  const automationRuns = useMemo(() => pickAutomationRuns(automationRunsQ.data), [automationRunsQ.data]);
  const automationRunsPreview = automationRuns.slice(0, 6);
  const automationCounts = useMemo(() => {
    return automationObjects.reduce(
      (acc, item) => {
        const kind = (item.kind || "").toLowerCase();
        if (kind === "task") acc.tasks += 1;
        if (kind === "watcher") acc.watchers += 1;
        if (kind === "app") acc.apps += 1;
        if (kind === "integration") acc.integrations += 1;
        return acc;
      },
      { tasks: 0, watchers: 0, apps: 0, integrations: 0 }
    );
  }, [automationObjects]);

  const currentTask = useMemo(() => {
    const inProgress = tasks.find((t) => {
      const s = String(t?.status || "").toLowerCase();
      return s.includes("inprogress");
    });
    return inProgress?.description;
  }, [tasks]);

  // Check if LLM is configured from settings
  const hasLlmConfigured = useMemo(() => {
    if (!settingsQ.data) return true; // Assume OK while loading
    const settings = settingsQ.data as Record<string, unknown>;
    // Check various possible fields for LLM configuration
    const pool = settings.model_pool || settings.llm_pool || settings.models;
    if (Array.isArray(pool)) return pool.length > 0;
    const provider = settings.llm_provider || settings.provider;
    if (provider && String(provider).trim()) return true;
    const apiKey = settings.openai_api_key || settings.anthropic_api_key || settings.api_key;
    if (apiKey && String(apiKey).trim()) return true;
    // If we got settings but no LLM-related fields exist, it might be structured differently
    // Be conservative: only flag if settings loaded successfully and look clearly empty
    return Object.keys(settings).length === 0 ? false : true;
  }, [settingsQ.data]);

  const autonomySettings = useMemo(() => {
    const root = asRecord(autonomySettingsQ.data);
    return asRecord(root.settings);
  }, [autonomySettingsQ.data]);

  const agentPaused = Boolean(autonomySettings.agent_paused ?? false);
  const pauseScopeItems = [
    "Scheduled tasks",
    "Watchers",
    "ArkPulse runs",
    "Autopilot/background analysis",
    "Proactive outbound notifications",
  ];

  // --- Mutations ---
  const approveMutation = useMutation({
    mutationFn: api.approveTask,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tasks"] }),
  });
  const rejectMutation = useMutation({
    mutationFn: api.rejectTask,
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["tasks"] }),
  });
  const retryMutation = useMutation({
    mutationFn: api.retryTask,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });
  const executeSkillMutation = useMutation({
    mutationFn: api.executeRecommendedSkill,
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["nudges"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });
  const nudgeFeedbackMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "dismiss" | "snooze" }) =>
      api.feedbackNudge(id, { action, snooze_minutes: action === "snooze" ? 24 * 60 : undefined }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ["nudges"] }),
  });
  const runBriefingMutation = useMutation({
    mutationFn: () =>
      api.executeRecommendedSkill({
        id: "daily_brief_now",
        title: "Run Daily Brief",
        skill_kind: "daily_brief_now",
        payload: {},
      } as RecommendedSkill),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });
  const pauseMutation = useMutation({
    mutationFn: (nextPaused: boolean) =>
      api.rawPost("/autonomy/settings", {
        agent_paused: nextPaused,
        pause_mode: "autonomous_only",
      }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings-dashboard"] });
      await queryClient.invalidateQueries({ queryKey: ["autonomy-settings"] });
      await queryClient.invalidateQueries({ queryKey: ["briefing"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks"] });
      await queryClient.invalidateQueries({ queryKey: ["trace"] });
    },
  });

  async function handleTogglePause() {
    if (pauseMutation.isPending) return;
    const nextPaused = !agentPaused;
    setPauseDialogOpen(true);
    setPauseError(null);
    setPauseTarget(nextPaused ? "pause" : "resume");
    setPausePhase(nextPaused ? "stopping" : "resuming");
    try {
      await pauseMutation.mutateAsync(nextPaused);
      setPausePhase(nextPaused ? "stopped" : "resumed");
    } catch (error) {
      setPauseError(errMessage(error));
      setPausePhase("idle");
    }
  }

  const hasErrors = !!(
    tasksQ.error ||
    traceQ.error ||
    briefingQ.error ||
    autonomySettingsQ.error ||
    automationQ.error ||
    automationRunsQ.error
  );
  const failingSources = [
    tasksQ.error ? "tasks" : null,
    traceQ.error ? "trace" : null,
    briefingQ.error ? "briefing" : null,
    autonomySettingsQ.error ? "autonomy settings" : null,
    automationQ.error ? "automation objects" : null,
    automationRunsQ.error ? "automation runs" : null,
  ].filter(Boolean) as string[];
  const dataSourceErrorSummary =
    failingSources.length === 0
      ? ""
      : failingSources.length === 1
        ? `${failingSources[0]} failed to load. Retrying automatically.`
        : `${failingSources.join(", ")} failed to load. Retrying automatically.`;

  return (
    <Box
      data-tour-target="overview-dashboard"
      className="overview-shell"
    >
      <Box data-tour-target="welcome-hero">
        <WelcomeHero
          onGoChat={() => navigateToView("chat")}
          onRunBriefing={() => runBriefingMutation.mutate()}
          onViewTasks={() => navigateToView("tasks")}
          onTogglePause={() => {
            void handleTogglePause();
          }}
          agentPaused={agentPaused}
          briefingLoading={runBriefingMutation.isPending}
          pauseLoading={pauseMutation.isPending}
        />
      </Box>

      {hasErrors ? (
        <Alert severity="error">
          {dataSourceErrorSummary}
        </Alert>
      ) : null}

      <Box className="overview-stage">
        <NeedsAttentionInbox
          tasks={tasks}
          notifications={notifications}
          securityLogs={securityLogs}
          settingsLoaded={!settingsQ.isLoading}
          hasLlmConfigured={hasLlmConfigured}
          onApprove={(id) => approveMutation.mutate(id)}
          onReject={(id) => rejectMutation.mutate(id)}
          onRetry={(id) => retryMutation.mutate(id)}
          onNavigate={navigateToView}
          approving={approveMutation.isPending}
          rejecting={rejectMutation.isPending}
          retrying={retryMutation.isPending}
        />

        <Stack className="overview-command-stack">
          <AgentStatusBar
            serverStatus={serverStatus}
            serverError={serverError}
            serverLoading={serverLoading}
            currentTaskDesc={currentTask}
          />

          <Box className="overview-action-card">
            <Stack spacing={1.25}>
              <Box>
                <Typography
                  variant="overline"
                  sx={{
                    color: "rgba(140, 190, 236, 0.8)",
                    letterSpacing: "0.12em",
                    display: "block",
                    mb: 0.35
                  }}
                >
                  Live Surface
                </Typography>
                <Typography variant="h6" sx={{ fontWeight: 600, mb: 0.45 }}>
                  Keep the first screen focused.
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  Open the deeper operational views only when you need them. The landing view should stay clean and readable.
                </Typography>
              </Box>

              <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap">
                <Chip size="small" label={`${automationCounts.tasks} tasks`} />
                <Chip size="small" label={`${automationCounts.watchers} watchers`} />
                <Chip size="small" label={`${automationCounts.apps} apps`} />
                <Chip size="small" label={`${automationCounts.integrations} integrations`} />
                <Chip size="small" label={`${traces.length} traces`} />
              </Stack>

              <Stack direction={{ xs: "column", sm: "row" }} spacing={1}>
                <Button variant="outlined" size="small" onClick={() => setInventoryOpen(true)}>
                  Automation Inventory
                </Button>
                <Button variant="outlined" size="small" onClick={() => setActivityOpen(true)}>
                  Recent Activity
                </Button>
                <Button variant="text" size="small" onClick={() => navigateToView("trace")}>
                  Open Trace
                </Button>
              </Stack>
            </Stack>
          </Box>
        </Stack>
      </Box>

      <Grid2 container spacing={1.2} className="overview-secondary-grid">
        <Grid2 size={{ xs: 12, lg: 7 }}>
          <TodaysHighlights tasks={tasks} traces={traces} />
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 5 }}>
          <SmartSuggestions
            briefing={briefingQ.data}
            nudges={nudges}
            onExecuteSkill={(skill) => executeSkillMutation.mutate(skill)}
            onSnooze={(id) => nudgeFeedbackMutation.mutate({ id, action: "snooze" })}
            onDismiss={(id) => nudgeFeedbackMutation.mutate({ id, action: "dismiss" })}
            executing={executeSkillMutation.isPending}
            feedbackPending={nudgeFeedbackMutation.isPending}
          />
        </Grid2>
      </Grid2>

      {/* Automation Inventory Dialog */}
      <Dialog
        open={inventoryOpen}
        onClose={() => setInventoryOpen(false)}
        maxWidth="md"
        fullWidth
        PaperProps={{
          sx: {
            background: "rgba(10, 15, 28, 0.97)",
            border: "1px solid rgba(47, 212, 255, 0.18)",
            backdropFilter: "blur(20px)",
          },
        }}
      >
        <DialogTitle sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <Box>
            <Typography variant="h6">Automation Inventory</Typography>
            <Typography variant="body2" color="text.secondary">
              Unified runtime view of active tasks, watchers, deployed apps, and integrations.
            </Typography>
          </Box>
          <IconButton size="small" onClick={() => setInventoryOpen(false)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          <Stack direction="row" spacing={0.75} useFlexGap flexWrap="wrap" mb={2}>
            <Chip size="small" label={`${automationCounts.tasks} tasks`} />
            <Chip size="small" label={`${automationCounts.watchers} watchers`} />
            <Chip size="small" label={`${automationCounts.apps} apps`} />
            <Chip size="small" label={`${automationCounts.integrations} integrations`} />
          </Stack>

          {automationQ.error ? (
            <Alert severity="error">{errMessage(automationQ.error)}</Alert>
          ) : automationPreview.length === 0 ? (
            <Typography variant="body2" color="text.secondary">
              No active automation objects yet.
            </Typography>
          ) : (
            <Stack spacing={1} mb={3}>
              {automationPreview.map((item) => (
                <Box key={`${item.kind}-${item.id}`} className="action-row">
                  <Stack direction="row" justifyContent="space-between" alignItems="flex-start" spacing={1.25}>
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Stack direction="row" spacing={0.75} alignItems="center" useFlexGap flexWrap="wrap">
                        <Chip size="small" label={automationKindLabel(item.kind)} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={item.title}>
                          {item.title}
                        </Typography>
                      </Stack>
                      {item.subtitle ? (
                        <Typography variant="caption" color="text.secondary" noWrap title={item.subtitle}>
                          {item.subtitle}
                        </Typography>
                      ) : null}
                      {item.detail ? (
                        <Typography variant="caption" color="text.secondary" noWrap title={item.detail}>
                          {item.detail}
                        </Typography>
                      ) : null}
                      {item.next_run_at ? (
                        <Typography variant="caption" color="text.secondary">
                          Next run: {formatAutomationTime(item.next_run_at)}
                        </Typography>
                      ) : null}
                    </Stack>
                    <Stack direction="row" spacing={1} alignItems="center" sx={{ flexShrink: 0 }}>
                      <Chip size="small" label={item.status || "-"} color={automationStatusColor(item.status || "")} />
                      {item.url ? (
                        <Button
                          size="small"
                          onClick={() => window.open(item.url || "", "_blank", "noopener,noreferrer")}
                        >
                          Open
                        </Button>
                      ) : null}
                      <Button
                        size="small"
                        onClick={() => navigateToView(targetViewForAutomation(item))}
                      >
                        View
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}

          {/* Recent Automation Runs subsection */}
          <Typography variant="h6" mb={0.5}>Recent Automation Runs</Typography>
          <Typography variant="body2" color="text.secondary" mb={1.5}>
            Supervisor history for background tasks and watchers, including retries and validation summaries.
          </Typography>

          {automationRunsQ.error ? (
            <Alert severity="error">{errMessage(automationRunsQ.error)}</Alert>
          ) : automationRunsPreview.length === 0 ? (
            <Typography variant="body2" color="text.secondary">
              No automation runs recorded yet.
            </Typography>
          ) : (
            <Stack spacing={1}>
              {automationRunsPreview.map((item) => (
                <Box key={item.id} className="action-row">
                  <Stack direction="row" justifyContent="space-between" alignItems="flex-start" spacing={1.25}>
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Stack direction="row" spacing={0.75} alignItems="center" useFlexGap flexWrap="wrap">
                        <Chip size="small" label={automationKindLabel(item.kind)} />
                        <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={item.title}>
                          {item.title}
                        </Typography>
                        <Chip size="small" label={`Attempt ${item.attempt}`} />
                      </Stack>
                      <Typography variant="caption" color="text.secondary" noWrap title={item.summary}>
                        {item.summary}
                      </Typography>
                      <Typography variant="caption" color="text.secondary">
                        Started: {formatAutomationTime(item.started_at)}
                        {item.next_retry_at ? ` | Next retry: ${formatAutomationTime(item.next_retry_at)}` : ""}
                      </Typography>
                    </Stack>
                    <Stack direction="row" spacing={1} alignItems="center" sx={{ flexShrink: 0 }}>
                      <Chip
                        size="small"
                        label={item.current_status || item.status || "-"}
                        color={automationStatusColor(item.current_status || item.status || "")}
                      />
                      <Button
                        size="small"
                        onClick={() => navigateToView(targetViewForAutomationRun(item))}
                      >
                        View
                      </Button>
                    </Stack>
                  </Stack>
                </Box>
              ))}
            </Stack>
          )}
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setInventoryOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>

      {/* Recent Activity Dialog */}
      <Dialog
        open={activityOpen}
        onClose={() => setActivityOpen(false)}
        maxWidth="md"
        fullWidth
        PaperProps={{
          sx: {
            background: "rgba(10, 15, 28, 0.97)",
            border: "1px solid rgba(47, 212, 255, 0.18)",
            backdropFilter: "blur(20px)",
          },
        }}
      >
        <DialogTitle sx={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <Typography variant="h6">Recent Activity</Typography>
          <IconButton size="small" onClick={() => setActivityOpen(false)}>
            <CloseIcon fontSize="small" />
          </IconButton>
        </DialogTitle>
        <DialogContent dividers>
          <ActivityFeed
            traces={traces}
            onViewAll={() => {
              setActivityOpen(false);
              navigateToView("trace");
            }}
          />
        </DialogContent>
        <DialogActions>
          <Button onClick={() => { setActivityOpen(false); navigateToView("trace"); }}>
            View All Traces
          </Button>
          <Button onClick={() => setActivityOpen(false)}>Close</Button>
        </DialogActions>
      </Dialog>

      <Dialog
        open={pauseDialogOpen}
        onClose={() => {
          if (pauseMutation.isPending) return;
          setPauseDialogOpen(false);
          setPauseError(null);
          setPausePhase("idle");
          setPauseTarget(null);
        }}
        maxWidth="sm"
        fullWidth
      >
        <DialogTitle>
          {pausePhase === "stopping" && "Pausing Agent"}
          {pausePhase === "stopped" && "Agent Paused"}
          {pausePhase === "resuming" && "Resuming Agent"}
          {pausePhase === "resumed" && "Agent Resumed"}
          {pausePhase === "idle" && (pauseTarget === "resume" ? "Resume Agent" : "Pause Agent")}
        </DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            {pausePhase === "stopping" || pausePhase === "resuming" ? (
              <Stack direction="row" spacing={1} alignItems="center">
                <CircularProgress size={18} />
                <Typography variant="body2">
                  {pausePhase === "stopping"
                    ? "stopping: disabling autonomous background activity..."
                    : "resuming: re-enabling autonomous background activity..."}
                </Typography>
              </Stack>
            ) : null}

            {pausePhase === "stopped" || pausePhase === "resumed" ? (
              <Chip
                size="small"
                color="success"
                label={pausePhase === "stopped" ? "stopped" : "resumed"}
                sx={{ width: "fit-content" }}
              />
            ) : null}

            <Typography variant="body2" color="text.secondary">
              {pauseTarget === "pause"
                ? "When paused, these systems are suspended:"
                : "On resume, these systems are active again:"}
            </Typography>

            <Stack spacing={0.5}>
              {pauseScopeItems.map((item) => (
                <Typography key={item} variant="body2">
                  - {item}
                </Typography>
              ))}
            </Stack>

            {pauseError ? <Alert severity="error">{pauseError}</Alert> : null}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              setPauseDialogOpen(false);
              setPauseError(null);
              setPausePhase("idle");
              setPauseTarget(null);
            }}
            disabled={pauseMutation.isPending}
          >
            Close
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}
