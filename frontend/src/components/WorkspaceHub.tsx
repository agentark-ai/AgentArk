import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import {
  Box,
  Button,
  Card,
  CardContent,
  Chip,
  Divider,
  Drawer,
  IconButton,
  Stack,
  Typography,
} from "@mui/material";
import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { formatUiDateTime } from "../lib/dateFormat";
import type { Task, TraceSummary } from "../types";
import { NativeWorkspace, type WorkspaceView } from "./NativeWorkspace";

const REFRESH_MS = 8000;

type DrawerView = Extract<
  WorkspaceView,
  "tasks" | "apps" | "documents" | "skills" | "swarm" | "trace" | "status" | "goals" | "moltbook"
>;

type Props = {
  autoRefresh: boolean;
  showAdvanced: boolean;
  unreadCount: number;
  onNavigateToView: (view: string, replace?: boolean) => void;
};

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? (value as Record<string, unknown>) : {};
}

function pickTasks(value: unknown): Task[] {
  if (Array.isArray(value)) return value as Task[];
  const record = asRecord(value);
  return Array.isArray(record.tasks) ? (record.tasks as Task[]) : [];
}

function pickTraceHistory(value: unknown): TraceSummary[] {
  const record = asRecord(value);
  return Array.isArray(record.history) ? (record.history as TraceSummary[]) : [];
}

function pickRecords(value: unknown, key: string): Record<string, unknown>[] {
  const record = asRecord(value);
  return Array.isArray(record[key]) ? (record[key] as Record<string, unknown>[]) : [];
}

function taskStatusKey(task: Task): string {
  return String(task?.status || "").toLowerCase();
}

function formatTaskStatus(task: Task): string {
  const value = taskStatusKey(task);
  if (value.includes("awaitingapproval")) return "Awaiting approval";
  if (value.includes("inprogress")) return "Running";
  if (value.includes("paused")) return "Paused";
  if (value.includes("failed")) return "Failed";
  if (value.includes("completed")) return "Completed";
  return value || "Pending";
}

function formatWhen(raw?: string): string {
  return formatUiDateTime(raw, { fallback: "-" });
}

const DRAWER_VIEWS: Array<{ view: DrawerView; label: string; detail: string }> = [
  { view: "tasks", label: "Tasks", detail: "Durable execution queue and approvals." },
  { view: "apps", label: "Apps", detail: "Built artifacts and deployed surfaces." },
  { view: "documents", label: "Files", detail: "Knowledge, uploads, and project documents." },
  { view: "skills", label: "Skills", detail: "Reusable capabilities and imports." },
  { view: "swarm", label: "Agents", detail: "Specialist agents and live roster." },
  { view: "trace", label: "Trace", detail: "Execution history and tool telemetry." },
  { view: "status", label: "Watchers", detail: "Background monitors and triggers." },
  { view: "goals", label: "Goals", detail: "Long-running intent and outcomes." },
  { view: "moltbook", label: "Moltbook", detail: "Community exposure and publishing." },
];

export function WorkspaceHub({
  autoRefresh,
  showAdvanced,
  unreadCount,
  onNavigateToView,
}: Props) {
  const interval = autoRefresh ? REFRESH_MS : false;
  const [drawerView, setDrawerView] = useState<DrawerView | null>(null);

  const tasksQ = useQuery({
    queryKey: ["tasks"],
    queryFn: api.getTasks,
    refetchInterval: interval,
  });
  const traceQ = useQuery({
    queryKey: ["trace"],
    queryFn: api.getTrace,
    refetchInterval: interval,
  });
  const projectsQ = useQuery({
    queryKey: ["workspace-projects"],
    queryFn: () => api.rawGet("/projects"),
    refetchInterval: interval,
  });
  const appsQ = useQuery({
    queryKey: ["apps-manager"],
    queryFn: () => api.rawGet("/apps"),
    refetchInterval: interval,
  });

  const tasks = useMemo(() => pickTasks(tasksQ.data), [tasksQ.data]);
  const traces = useMemo(() => pickTraceHistory(traceQ.data), [traceQ.data]);
  const projects = useMemo(() => pickRecords(projectsQ.data, "projects"), [projectsQ.data]);
  const appsPayload = useMemo(() => asRecord(appsQ.data), [appsQ.data]);
  const apps = useMemo(() => pickRecords(appsPayload, "apps"), [appsPayload]);
  const restoreInfo = useMemo(() => asRecord(appsPayload.restore), [appsPayload]);
  const restoreActive = String(restoreInfo.active || "").toLowerCase() === "true";
  const restoringApps = useMemo(
    () =>
      apps.filter((app) => {
        const restoring = app.restoring;
        const status = String(app.restore_status || "").toLowerCase();
        return restoring === true || String(restoring).toLowerCase() === "true" || status === "restoring";
      }),
    [apps]
  );
  const degradedApps = useMemo(
    () =>
      apps.filter((app) => {
        const status = String(app.restore_status || "").toLowerCase();
        return status === "degraded";
      }),
    [apps]
  );

  const runningTasks = useMemo(
    () => tasks.filter((task) => taskStatusKey(task).includes("inprogress")),
    [tasks]
  );
  const waitingTasks = useMemo(
    () =>
      tasks.filter((task) => {
        const status = taskStatusKey(task);
        return status.includes("awaitingapproval") || status.includes("paused");
      }),
    [tasks]
  );
  const failedTasks = useMemo(
    () => tasks.filter((task) => taskStatusKey(task).includes("failed")),
    [tasks]
  );
  const activeApps = useMemo(
    () =>
      apps.filter((app) => {
        const running = app.running;
        return running === true || String(running).toLowerCase() === "true";
      }),
    [apps]
  );

  const liveTaskPreview = useMemo(
    () => [...runningTasks, ...waitingTasks, ...failedTasks].slice(0, 4),
    [failedTasks, runningTasks, waitingTasks]
  );
  const latestTrace = traces[0];
  const drawerMeta = DRAWER_VIEWS.find((entry) => entry.view === drawerView) || null;

  useEffect(() => {
    if (autoRefresh || (!restoreActive && restoringApps.length === 0)) return;
    const timer = setInterval(() => {
      void appsQ.refetch();
    }, 1500);
    return () => clearInterval(timer);
  }, [autoRefresh, restoreActive, restoringApps.length, appsQ]);

  return (
    <Box className="workspace-hub-shell" data-tour-target="workspace-shell">
      <Box className="workspace-launch-bar">
        <Stack
          direction={{ xs: "column", lg: "row" }}
          spacing={1.5}
          sx={{
            justifyContent: "space-between",
            alignItems: { xs: "flex-start", lg: "center" }
          }}>
          <Box sx={{ minWidth: 0 }}>
            <Typography variant="overline" className="workspace-shell-kicker">
              Active Workspace
            </Typography>
            <Typography variant="h5" sx={{ fontWeight: 700, letterSpacing: 0, mb: 0.35 }}>
              Ask naturally. Deeper work stays one click away.
            </Typography>
            <Typography
              variant="body2"
              sx={{
                color: "text.secondary",
                maxWidth: 860
              }}>
              Ask quick questions directly. When the work needs files, tools, approvals, apps, or repeatability, the run
              stays visible as a task without sending you to a different product surface.
            </Typography>
          </Box>
          <Stack direction="row" spacing={0.75} useFlexGap sx={{
            flexWrap: "wrap"
          }}>
            {DRAWER_VIEWS.slice(0, 6).map((entry) => (
              <Button
                key={entry.view}
                variant={drawerView === entry.view ? "contained" : "outlined"}
                size="small"
                onClick={() => setDrawerView(entry.view)}
              >
                {entry.label}
              </Button>
            ))}
            <Button
              size="small"
              variant="text"
              onClick={() => onNavigateToView("projects")}
            >
              Projects
            </Button>
          </Stack>
        </Stack>
      </Box>
      <Box className="workspace-hub-grid">
        <Box className="workspace-chat-stage">
          <NativeWorkspace
            view="chat"
            autoRefresh={autoRefresh}
            showAdvanced={showAdvanced}
            onNavigateToView={onNavigateToView}
          />
        </Box>

        <Stack className="workspace-companion-rail" spacing={1.1}>
          <Card className="workspace-side-card">
            <CardContent sx={{ p: 1.5 }}>
              <Stack spacing={1.1}>
                <Box>
                  <Typography variant="overline" className="workspace-side-kicker">
                    Task Flow
                  </Typography>
                  <Typography variant="h6" sx={{ fontWeight: 650 }}>
                    Live execution stays visible.
                  </Typography>
                </Box>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  <Chip size="small" color="info" label={`${runningTasks.length} running`} />
                  <Chip size="small" color="warning" label={`${waitingTasks.length} waiting`} />
                  <Chip size="small" color={failedTasks.length > 0 ? "error" : "default"} label={`${failedTasks.length} failed`} />
                </Stack>
                {liveTaskPreview.length === 0 ? (
                  <Typography variant="body2" sx={{
                    color: "text.secondary"
                  }}>
                    No active or blocked tasks right now. The thread is clear for quick asks.
                  </Typography>
                ) : (
                  <Stack spacing={0.75}>
                    {liveTaskPreview.map((task) => (
                      <Box key={task.id} className="action-row">
                        <Stack spacing={0.45}>
                          <Stack
                            direction="row"
                            spacing={0.75}
                            useFlexGap
                            sx={{
                              alignItems: "center",
                              flexWrap: "wrap"
                            }}>
                            <Chip size="small" variant="outlined" label={formatTaskStatus(task)} />
                            <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={task.description}>
                              {task.description || "Task"}
                            </Typography>
                          </Stack>
                          <Typography variant="caption" sx={{
                            color: "text.secondary"
                          }}>
                            Created {formatWhen(task.created_at)}
                          </Typography>
                        </Stack>
                      </Box>
                    ))}
                  </Stack>
                )}
                <Stack direction="row" spacing={1}>
                  <Button size="small" variant="outlined" onClick={() => setDrawerView("tasks")} sx={{ textTransform: "none" }}>
                    Review Tasks
                  </Button>
                  <Button size="small" variant="text" onClick={() => onNavigateToView("overview")} sx={{ textTransform: "none" }}>
                    Mission Control
                  </Button>
                </Stack>
              </Stack>
            </CardContent>
          </Card>

          <Card className="workspace-side-card">
            <CardContent sx={{ p: 1.5 }}>
              <Stack spacing={1.1}>
                <Box>
                  <Typography variant="overline" className="workspace-side-kicker">
                    Context
                  </Typography>
                  <Typography variant="h6" sx={{ fontWeight: 650 }}>
                    Projects, artifacts, and recent runs.
                  </Typography>
                </Box>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  <Chip size="small" label={`${projects.length} projects`} />
                  <Chip size="small" label={`${activeApps.length} live apps`} />
                  {restoringApps.length > 0 ? (
                    <Chip size="small" color="info" label={`${restoringApps.length} restoring`} />
                  ) : null}
                  {degradedApps.length > 0 ? (
                    <Chip size="small" color="warning" label={`${degradedApps.length} need review`} />
                  ) : null}
                  <Chip size="small" label={`${traces.length} traces`} />
                    <Chip size="small" color={unreadCount > 0 ? "warning" : "default"} label={`${unreadCount} alerts`} />
                </Stack>
                <Divider sx={{ borderColor: "rgba(108, 156, 212, 0.12)" }} />
                <Stack spacing={0.65}>
                  <Typography variant="caption" sx={{
                    color: "text.secondary"
                  }}>
                    Latest run
                  </Typography>
                  <Typography variant="body2" sx={{ fontWeight: 600 }} noWrap title={latestTrace?.message_preview || ""}>
                    {latestTrace?.message_preview || "No recent traces yet."}
                  </Typography>
                  {latestTrace ? (
                    <Typography variant="caption" sx={{
                      color: "text.secondary"
                    }}>
                      {latestTrace.status || "unknown"} • {latestTrace.step_count} step{latestTrace.step_count === 1 ? "" : "s"} • {formatWhen(latestTrace.started_at)}
                    </Typography>
                  ) : null}
                </Stack>
                <Stack direction="row" spacing={1} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  <Button size="small" variant="outlined" onClick={() => onNavigateToView("projects")} sx={{ textTransform: "none" }}>
                    Open Projects
                  </Button>
                  <Button size="small" variant="outlined" onClick={() => setDrawerView("trace")} sx={{ textTransform: "none" }}>
                    Open Trace
                  </Button>
                  <Button size="small" variant="text" onClick={() => onNavigateToView("overview")} sx={{ textTransform: "none" }}>
                    Home
                  </Button>
                </Stack>
              </Stack>
            </CardContent>
          </Card>

          <Card className="workspace-side-card">
            <CardContent sx={{ p: 1.5 }}>
              <Stack spacing={1.1}>
                <Box>
                  <Typography variant="overline" className="workspace-side-kicker">
                    Surfaces
                  </Typography>
                  <Typography variant="h6" sx={{ fontWeight: 650 }}>
                    Open deeper tools only when needed.
                  </Typography>
                </Box>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{
                  flexWrap: "wrap"
                }}>
                  {DRAWER_VIEWS.map((entry) => (
                    <Chip
                      key={entry.view}
                      label={entry.label}
                      clickable
                      variant={drawerView === entry.view ? "filled" : "outlined"}
                      color={drawerView === entry.view ? "primary" : "default"}
                      onClick={() => setDrawerView(entry.view)}
                    />
                  ))}
                </Stack>
                <Typography variant="body2" sx={{
                  color: "text.secondary"
                }}>
                  This keeps the first screen focused while still exposing the full assistant workspace behind one action.
                </Typography>
                <Stack direction="row" spacing={1}>
                  <Button size="small" variant="outlined" onClick={() => onNavigateToView("library")} sx={{ textTransform: "none" }}>
                    Library
                  </Button>
                  <Button size="small" variant="text" onClick={() => onNavigateToView("overview")} sx={{ textTransform: "none" }}>
                    Mission Control
                  </Button>
                </Stack>
              </Stack>
            </CardContent>
          </Card>
        </Stack>
      </Box>
      <Drawer
        anchor="right"
        open={drawerView !== null}
        onClose={() => setDrawerView(null)}
        slotProps={{
          paper: {
            sx: {
              width: { xs: "100%", md: 640 },
              maxWidth: "100vw",
              borderLeft: "1px solid rgba(108, 156, 212, 0.18)",
              background: "linear-gradient(160deg, rgba(9, 21, 39, 0.97), rgba(7, 16, 30, 0.9))",
            },
          }
        }}
      >
        <Box className="workspace-side-drawer">
          <Stack
            direction="row"
            spacing={1}
            sx={{
              alignItems: "center",
              justifyContent: "space-between",
              px: 1.5,
              py: 1.2,
              borderBottom: "1px solid rgba(108, 156, 212, 0.14)"
            }}>
            <Box sx={{ minWidth: 0 }}>
              <Typography variant="subtitle1" sx={{ fontWeight: 650 }} noWrap>
                {drawerMeta?.label || "Workspace panel"}
              </Typography>
              <Typography variant="caption" sx={{
                color: "text.secondary"
              }}>
                {drawerMeta?.detail || "Assistant workspace"}
              </Typography>
            </Box>
            <IconButton size="small" onClick={() => setDrawerView(null)} aria-label="Close workspace drawer">
              <CloseRoundedIcon fontSize="small" />
            </IconButton>
          </Stack>
          <Box sx={{ flex: 1, minHeight: 0 }}>
            {drawerView ? (
              <NativeWorkspace
                view={drawerView}
                autoRefresh={autoRefresh}
                showAdvanced={showAdvanced}
                onNavigateToView={onNavigateToView}
              />
            ) : null}
          </Box>
        </Box>
      </Drawer>
    </Box>
  );
}
