import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  Tab,
  Tabs,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { api } from "../../api/client";
import {
  formatUiDateOnly,
  formatUiRelativeDateTimeMeta,
} from "../../lib/dateFormat";
import { WorkspacePageHeader, WorkspacePageShell } from "../WorkspacePage";
import {
  type JsonRecord,
  asRecord,
  errMessage,
  num,
  pickRecords,
  str,
} from "./pageHelpers";

const REFRESH_MS = 8000;
const HEADER_ACTION_GROUP_SX = {
  p: 0.45,
  borderRadius: "8px",
  border: "1px solid var(--surface-border)",
  background: "var(--ui-rgba-255-255-255-020)",
  boxShadow: "inset 0 1px 0 var(--ui-rgba-255-255-255-030)",
} as const;
const HEADER_PRIMARY_BUTTON_SX = {
  minHeight: 32,
  px: 1.5,
  borderRadius: "8px",
  fontWeight: 700,
  textTransform: "none",
  boxShadow: "none",
} as const;

function humanTs(raw: string): { label: string; tip: string } {
  return formatUiRelativeDateTimeMeta(raw, { fallback: "-" });
}

type GoalsPageProps = {
  autoRefresh: boolean;
};

export default function GoalsPage({ autoRefresh }: GoalsPageProps) {
  const queryClient = useQueryClient();
  type GoalLoopPayload = {
    goal: string;
    constraints?: string;
    due_date?: string;
    report_cron?: string;
    preview_only?: boolean;
    plan_override?: JsonRecord;
  };
  const [description, setDescription] = useState("");
  const [dueDate, setDueDate] = useState("");
  const [autopilotEnabled, setAutopilotEnabled] = useState(true);
  const [guardrails, setGuardrails] = useState("");
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [scheduleKey, setScheduleKey] = useState("daily_9");
  const [reportCron, setReportCron] = useState("0 0 9 * * *"); // 09:00 daily (UTC unless server uses user tz)
  const [selectedGoalId, setSelectedGoalId] = useState<string | null>(null); // goal_id from arguments
  const [planPreview, setPlanPreview] = useState<JsonRecord | null>(null);
  const [goalCreateOpen, setGoalCreateOpen] = useState(false);
  const [goalConfirmOpen, setGoalConfirmOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const schedulePresets: {
    key: string;
    label: string;
    cron: string | null;
    hint?: string;
  }[] = [
    { key: "run_5", label: "Every 5 minutes", cron: "0 */5 * * * *" },
    { key: "run_10", label: "Every 10 minutes", cron: "0 */10 * * * *" },
    { key: "run_30", label: "Every 30 minutes", cron: "0 */30 * * * *" },
    { key: "hourly", label: "Hourly", cron: "0 0 * * * *" },
    { key: "daily_9", label: "Daily (09:00)", cron: "0 0 9 * * *" },
    { key: "weekly_mon_9", label: "Weekly (Mon 09:00)", cron: "0 0 9 * * 1" },
    { key: "monthly_1_9", label: "Monthly (1st 09:00)", cron: "0 0 9 1 * *" },
    {
      key: "custom",
      label: "Custom",
      cron: null,
      hint: "Cron uses 6 fields: sec min hour day month weekday",
    },
  ];
  const scheduleLabel = (key: string) => {
    for (const p of schedulePresets) {
      if (p.key === key) return p.label;
    }
    return "Custom";
  };

  const goalsQ = useQuery({
    queryKey: ["goals-list"],
    queryFn: () => api.rawGet("/goals?limit=100"),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const progressPath = selectedGoalId
    ? `/autonomy/goals/progress?goal_id=${encodeURIComponent(selectedGoalId)}`
    : "/autonomy/goals/progress";
  const progressQ = useQuery({
    queryKey: ["goals-progress", selectedGoalId],
    queryFn: () => api.rawGet(progressPath),
    refetchInterval: autoRefresh ? REFRESH_MS : false,
  });

  const createMutation = useMutation({
    mutationFn: (payload: { description: string; due_date?: string }) =>
      api.rawPost("/goals", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    },
  });

  const autopilotPreviewMutation = useMutation({
    mutationFn: (payload: GoalLoopPayload) =>
      api.rawPost("/autonomy/goals/loop", { ...payload, preview_only: true }),
  });

  const autopilotMutation = useMutation({
    mutationFn: (payload: GoalLoopPayload) =>
      api.rawPost("/autonomy/goals/loop", payload),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    },
  });

  const runNowMutation = useMutation({
    mutationFn: (goalId: string) =>
      api.rawPost("/autonomy/goals/report_now", { goal_id: goalId }),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) =>
      api.rawDelete(`/goals/${encodeURIComponent(id)}`),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["goals-list"] });
      await queryClient.invalidateQueries({ queryKey: ["goals-progress"] });
      await queryClient.invalidateQueries({ queryKey: ["tasks-manager"] });
    },
  });

  const summary = asRecord(asRecord(progressQ.data).summary);
  const goals = pickRecords(goalsQ.data, "goals");
  const progressItems = pickRecords(progressQ.data, "items");

  const examples = [
    "Build a weekly arXiv dashboard for RL + time series",
    "Ship a working prototype by Friday",
    "Audit the app for security issues and write a fix plan",
  ];

  const resetGoalDraft = (nextAutopilot: boolean, nextDescription = "") => {
    setDescription(nextDescription);
    setDueDate("");
    setGuardrails("");
    setScheduleKey("daily_9");
    setReportCron("0 0 9 * * *");
    setAdvancedOpen(false);
    setAutopilotEnabled(nextAutopilot);
    setGoalConfirmOpen(false);
    setPlanPreview(null);
    setError(null);
  };

  const openGoalDialog = (nextAutopilot = true, nextDescription = "") => {
    resetGoalDraft(nextAutopilot, nextDescription);
    setGoalCreateOpen(true);
  };

  const buildGoalLoopPayload = (): GoalLoopPayload => ({
    goal: description.trim(),
    constraints: guardrails.trim() || undefined,
    due_date: dueDate.trim() || undefined,
    report_cron: reportCron.trim() || undefined,
  });

  const submitGoalDraft = async () => {
    setError(null);
    try {
      const goalText = description.trim();
      if (autopilotEnabled) {
        if (!goalText) {
          setError("Goal is required.");
          return;
        }
        const previewOut = await autopilotPreviewMutation.mutateAsync(
          buildGoalLoopPayload(),
        );
        const preview = asRecord(asRecord(previewOut).plan_preview);
        setPlanPreview(Object.keys(preview).length ? preview : null);
        setGoalCreateOpen(false);
        setGoalConfirmOpen(true);
        return;
      } else {
        await createMutation.mutateAsync({
          description: goalText,
          due_date: dueDate.trim() || undefined,
        });
      }
      setGoalCreateOpen(false);
      resetGoalDraft(true);
    } catch (e) {
      setError(errMessage(e));
    }
  };

  const confirmAutopilotGoal = async () => {
    setError(null);
    try {
      const out = await autopilotMutation.mutateAsync({
        ...buildGoalLoopPayload(),
        plan_override: planPreview || undefined,
      });
      const gid = str(asRecord(out).goal_id, "");
      if (gid) setSelectedGoalId(gid);
      setGoalConfirmOpen(false);
      setGoalCreateOpen(false);
      resetGoalDraft(true);
    } catch (e) {
      setError(errMessage(e));
    }
  };

  return (
    <WorkspacePageShell spacing={1.5}>
      <WorkspacePageHeader
        eyebrow="Agent"
        title="Goals"
        description="Track outcomes and spin up AI autopilot loops when needed."
        actions={
          <Stack
            direction="row"
            spacing={0.75}
            useFlexGap
            sx={[
              {
                flexWrap: "wrap",
                alignItems: "center",
              },
              HEADER_ACTION_GROUP_SX,
            ]}
          >
            <Button
              size="small"
              variant="contained"
              sx={HEADER_PRIMARY_BUTTON_SX}
              onClick={() => openGoalDialog(true)}
            >
              Create Goal
            </Button>
          </Stack>
        }
      />
      <Box className="list-shell stat-strip">
        <div className="stat-strip-item">
          <span className="stat-strip-label">Autopilot Items</span>
          <span className="stat-strip-value">{num(summary.total)}</span>
          <span className="stat-strip-helper">Recent tasks tied to goals</span>
        </div>
        <div className="stat-strip-item">
          <span className="stat-strip-label">Completed</span>
          <span className="stat-strip-value">{num(summary.completed)}</span>
        </div>
        <div className="stat-strip-item">
          <span className="stat-strip-label">Pending/Running</span>
          <span className="stat-strip-value">
            {num(summary.pending_or_running)}
          </span>
        </div>
        <div className="stat-strip-item">
          <span className="stat-strip-label">Failed</span>
          <span className="stat-strip-value">{num(summary.failed)}</span>
        </div>
      </Box>
      <Grid2 container spacing={2}>
        <Grid2 size={{ xs: 12, lg: 6 }}>
          <Box className="list-shell">
            <Stack
              direction="row"
              sx={{
                justifyContent: "space-between",
                alignItems: "center",
                mb: 1,
              }}
            >
              <Typography variant="h6">Goals</Typography>
            </Stack>
            {goalsQ.error ? (
              <Alert severity="error">{errMessage(goalsQ.error)}</Alert>
            ) : goals.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No goals yet.
              </Typography>
            ) : (
              <Box className="metadata-box" sx={{ maxHeight: 520 }}>
                <Stack spacing={1}>
                  {goals.map((g) => {
                    const id = str(g.id, "");
                    const goalId = str(g.goal_id, "");
                    const hasAutopilot = g.autopilot === true && !!goalId;
                    const isSelected =
                      hasAutopilot && selectedGoalId === goalId;
                    const title =
                      str(g.goal, "").trim() ||
                      str(g.description, "Goal").replace(/^Goal:\\s*/i, "");
                    return (
                      <Box key={id} className="action-row">
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            justifyContent: "space-between",
                            alignItems: "center",
                          }}
                        >
                          <Button
                            variant="text"
                            size="small"
                            sx={{
                              justifyContent: "flex-start",
                              textAlign: "left",
                              flex: 1,
                              ...(isSelected
                                ? {
                                    border: "1px solid var(--ui-rgba-47-212-255-350)",
                                    background: "var(--ui-rgba-47-212-255-080)",
                                  }
                                : {}),
                            }}
                            onClick={() =>
                              setSelectedGoalId(
                                hasAutopilot
                                  ? isSelected
                                    ? null
                                    : goalId
                                  : null,
                              )
                            }
                          >
                            <Stack
                              spacing={0.3}
                              sx={{
                                alignItems: "flex-start",
                              }}
                            >
                              <Stack
                                direction="row"
                                spacing={1}
                                sx={{
                                  alignItems: "center",
                                }}
                              >
                                <Typography
                                  variant="body2"
                                  sx={{
                                    fontWeight: 700,
                                  }}
                                >
                                  {title}
                                </Typography>
                                {hasAutopilot ? (
                                  <Chip size="small" label="Autopilot" />
                                ) : (
                                  <Chip
                                    size="small"
                                    label="Manual"
                                    variant="outlined"
                                  />
                                )}
                              </Stack>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {str(g.status)}
                                {str(g.due_date)
                                  ? ` | due ${formatUiDateOnly(str(g.due_date), { fallback: str(g.due_date) })}`
                                  : ""}
                                {str(g.created_at) ? (
                                  <span
                                    title={humanTs(str(g.created_at)).tip}
                                  >{` | created ${humanTs(str(g.created_at)).label}`}</span>
                                ) : (
                                  ""
                                )}
                              </Typography>
                            </Stack>
                          </Button>
                          <Stack
                            direction="row"
                            spacing={1}
                            sx={{
                              alignItems: "center",
                            }}
                          >
                            {!hasAutopilot ? (
                              <Button
                                size="small"
                                disabled={autopilotMutation.isPending}
                                onClick={async () => {
                                  setError(null);
                                  setPlanPreview(null);
                                  try {
                                    const out =
                                      await autopilotMutation.mutateAsync({
                                        goal: title,
                                        due_date: str(g.due_date) || undefined,
                                        constraints:
                                          guardrails.trim() || undefined,
                                        report_cron:
                                          reportCron.trim() || undefined,
                                      });
                                    const newGoalId = str(
                                      asRecord(out).goal_id,
                                      "",
                                    );
                                    if (newGoalId) setSelectedGoalId(newGoalId);
                                  } catch (e) {
                                    setError(errMessage(e));
                                  }
                                }}
                              >
                                Start Autopilot
                              </Button>
                            ) : (
                              <Button
                                size="small"
                                onClick={() =>
                                  setSelectedGoalId(isSelected ? null : goalId)
                                }
                              >
                                {isSelected ? "Deselect" : "View"}
                              </Button>
                            )}
                            <Button
                              size="small"
                              color="error"
                              disabled={deleteMutation.isPending}
                              onClick={() => deleteMutation.mutate(id)}
                            >
                              Delete
                            </Button>
                          </Stack>
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            )}
          </Box>
        </Grid2>
        <Grid2 size={{ xs: 12, lg: 6 }}>
          <Box className="list-shell">
            <Stack
              direction="row"
              sx={{
                justifyContent: "space-between",
                alignItems: "center",
                mb: 1,
              }}
            >
              <Typography variant="h6">
                {selectedGoalId
                  ? "Autopilot Activity (selected goal)"
                  : "Autopilot Activity (all goals)"}
              </Typography>
              <Stack
                direction="row"
                spacing={1}
                sx={{
                  alignItems: "center",
                }}
              >
                {selectedGoalId ? (
                  <Button
                    size="small"
                    disabled={runNowMutation.isPending}
                    onClick={() => runNowMutation.mutate(selectedGoalId)}
                  >
                    Run now
                  </Button>
                ) : null}
                {selectedGoalId ? (
                  <Button size="small" onClick={() => setSelectedGoalId(null)}>
                    Clear
                  </Button>
                ) : null}
              </Stack>
            </Stack>
            {progressQ.error ? (
              <Alert severity="error">{errMessage(progressQ.error)}</Alert>
            ) : progressItems.length === 0 ? (
              <Typography
                variant="body2"
                sx={{
                  color: "text.secondary",
                }}
              >
                No goal-linked items yet.
              </Typography>
            ) : (
              <Box className="metadata-box" sx={{ maxHeight: 520 }}>
                <Stack spacing={1}>
                  {progressItems.map((it) => {
                    const id = str(it.id, "");
                    const status = str(it.status, "");
                    const statusColor = status.includes("Failed")
                      ? "error"
                      : status.includes("Completed")
                        ? "success"
                        : "warning";
                    return (
                      <Box key={id} className="action-row">
                        <Stack
                          direction="row"
                          spacing={1}
                          sx={{
                            justifyContent: "space-between",
                            alignItems: "center",
                          }}
                        >
                          <Stack spacing={0.3} sx={{ minWidth: 0 }}>
                            <Typography
                              variant="body2"
                              noWrap
                              sx={{
                                fontWeight: 700,
                              }}
                            >
                              {str(it.description, "Task")}
                            </Typography>
                            <Typography
                              variant="caption"
                              noWrap
                              sx={{
                                color: "text.secondary",
                              }}
                            >
                              {str(it.action)} |{" "}
                              <span title={humanTs(str(it.created_at)).tip}>
                                {humanTs(str(it.created_at)).label}
                              </span>
                            </Typography>
                          </Stack>
                          <Chip
                            size="small"
                            label={status || "Unknown"}
                            color={statusColor as any}
                          />
                        </Stack>
                      </Box>
                    );
                  })}
                </Stack>
              </Box>
            )}
          </Box>
        </Grid2>
      </Grid2>
      {error ? <Alert severity="error">{error}</Alert> : null}
      <Dialog
        open={goalCreateOpen}
        onClose={() => setGoalCreateOpen(false)}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Set a Goal</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Stack
              direction={{ xs: "column", sm: "row" }}
              sx={{
                justifyContent: "space-between",
                alignItems: { xs: "flex-start", sm: "center" },
              }}
            >
              <Typography
                variant="caption"
                sx={{
                  color: "text.secondary",
                }}
              >
                Use plain language. Autopilot enables AI planning and scheduled
                progress loops.
              </Typography>
              <FormControlLabel
                control={
                  <Switch
                    checked={autopilotEnabled}
                    onChange={(e) => setAutopilotEnabled(e.target.checked)}
                  />
                }
                label="Autopilot"
              />
            </Stack>
            <Grid2
              container
              spacing={1}
              sx={{
                alignItems: "stretch",
              }}
            >
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  label="What do you want to achieve?"
                  placeholder="Describe your goal in one sentence."
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  label="Due date (optional)"
                  placeholder="YYYY-MM-DD"
                  value={dueDate}
                  onChange={(e) => setDueDate(e.target.value)}
                />
              </Grid2>
              {autopilotEnabled ? (
                <Grid2 size={{ xs: 12 }}>
                  <TextField
                    fullWidth
                    size="small"
                    label="Guardrails (optional)"
                    placeholder="Example: Ask before deleting files. Keep it under 3 steps. No external posting."
                    value={guardrails}
                    onChange={(e) => setGuardrails(e.target.value)}
                  />
                </Grid2>
              ) : null}
              {autopilotEnabled ? (
                <Grid2 size={{ xs: 12 }}>
                  <Accordion
                    expanded={advancedOpen}
                    onChange={() => setAdvancedOpen((p) => !p)}
                    className="accordion-shell"
                  >
                    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Advanced
                      </Typography>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Stack spacing={1}>
                        <TextField
                          fullWidth
                          size="small"
                          select
                          label="Check-in schedule"
                          value={scheduleKey}
                          onChange={(e) => {
                            const next = e.target.value;
                            setScheduleKey(next);
                            let preset:
                              | (typeof schedulePresets)[number]
                              | undefined = undefined;
                            for (const p of schedulePresets) {
                              if (p.key === next) {
                                preset = p;
                                break;
                              }
                            }
                            if (preset && preset.cron)
                              setReportCron(preset.cron);
                          }}
                          helperText="When Autopilot is enabled, this schedules a periodic progress report task."
                        >
                          {schedulePresets.map((p) => (
                            <MenuItem key={p.key} value={p.key}>
                              {p.label}
                            </MenuItem>
                          ))}
                        </TextField>
                        {scheduleKey === "custom" ? (
                          <TextField
                            fullWidth
                            size="small"
                            label="Custom cron (6 fields)"
                            value={reportCron}
                            onChange={(e) => setReportCron(e.target.value)}
                            helperText={(() => {
                              for (const p of schedulePresets) {
                                if (p.key === "custom") return p.hint || "";
                              }
                              return "";
                            })()}
                          />
                        ) : (
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Selected: {scheduleLabel(scheduleKey)} ({reportCron}
                            )
                          </Typography>
                        )}
                      </Stack>
                    </AccordionDetails>
                  </Accordion>
                </Grid2>
              ) : null}
            </Grid2>
            <Stack
              direction="row"
              spacing={1}
              sx={{
                flexWrap: "wrap",
                opacity: 0.9,
              }}
            >
              {examples.map((ex) => (
                <Chip
                  key={ex}
                  size="small"
                  label={ex}
                  onClick={() => setDescription(ex)}
                  variant="outlined"
                  sx={{ mb: 0.5 }}
                />
              ))}
            </Stack>
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setGoalCreateOpen(false)}>Cancel</Button>
          <Button
            variant="contained"
            startIcon={
              autopilotEnabled && autopilotPreviewMutation.isPending ? (
                <CircularProgress size={14} color="inherit" />
              ) : undefined
            }
            disabled={
              !description.trim() ||
              createMutation.isPending ||
              autopilotMutation.isPending ||
              autopilotPreviewMutation.isPending
            }
            onClick={submitGoalDraft}
          >
            {autopilotEnabled
              ? autopilotPreviewMutation.isPending
                ? "Generating..."
                : "Create with AI"
              : "Save Goal"}
          </Button>
        </DialogActions>
      </Dialog>
      <Dialog
        open={goalConfirmOpen}
        onClose={() => {
          if (autopilotMutation.isPending) return;
          setGoalConfirmOpen(false);
        }}
        maxWidth="md"
        fullWidth
      >
        <DialogTitle>Confirm Goal Before Create</DialogTitle>
        <DialogContent dividers>
          <Stack spacing={1.25}>
            <Alert severity="info">
              AI has prepared a draft. Review and edit details before creating
              this goal.
            </Alert>
            <Grid2 container spacing={1}>
              <Grid2 size={{ xs: 12, md: 8 }}>
                <TextField
                  fullWidth
                  label="Goal"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12, md: 4 }}>
                <TextField
                  fullWidth
                  label="Due date (optional)"
                  placeholder="YYYY-MM-DD"
                  value={dueDate}
                  onChange={(e) => setDueDate(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Guardrails (optional)"
                  value={guardrails}
                  onChange={(e) => setGuardrails(e.target.value)}
                />
              </Grid2>
              <Grid2 size={{ xs: 12 }}>
                <TextField
                  fullWidth
                  size="small"
                  label="Report cron"
                  value={reportCron}
                  onChange={(e) => setReportCron(e.target.value)}
                  helperText="6-field cron expression for periodic progress reports."
                />
              </Grid2>
            </Grid2>

            {planPreview ? (
              <Stack spacing={1}>
                <TextField
                  fullWidth
                  size="small"
                  label="AI summary"
                  value={str(planPreview.summary, "")}
                  onChange={(e) =>
                    setPlanPreview((prev) =>
                      prev ? { ...prev, summary: e.target.value } : prev,
                    )
                  }
                />

                {Array.isArray(planPreview.steps) &&
                planPreview.steps.length > 0 ? (
                  <Stack spacing={1}>
                    {(planPreview.steps as unknown[])
                      .slice(0, 12)
                      .map((rawStep, idx) => {
                        const step = asRecord(rawStep);
                        const args = asRecord(step.arguments);
                        const argKeys = Object.keys(args);
                        const updateStepField = (
                          field: "title" | "action" | "why",
                          value: string,
                        ) => {
                          setPlanPreview((prev) => {
                            if (!prev) return prev;
                            const currentSteps = Array.isArray(prev.steps)
                              ? [...(prev.steps as unknown[])]
                              : [];
                            const existingStep = asRecord(currentSteps[idx]);
                            currentSteps[idx] = {
                              ...existingStep,
                              [field]: value,
                            };
                            return { ...prev, steps: currentSteps };
                          });
                        };
                        return (
                          <Box key={`goal-step-${idx}`} className="action-row">
                            <Stack spacing={0.8}>
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Step {idx + 1}
                              </Typography>
                              <TextField
                                fullWidth
                                size="small"
                                label="Title"
                                value={str(step.title, `Step ${idx + 1}`)}
                                onChange={(e) =>
                                  updateStepField("title", e.target.value)
                                }
                              />
                              <TextField
                                fullWidth
                                size="small"
                                label="Action"
                                value={str(step.action, "research")}
                                onChange={(e) =>
                                  updateStepField("action", e.target.value)
                                }
                              />
                              <TextField
                                fullWidth
                                size="small"
                                label="Why"
                                value={str(step.why, "")}
                                onChange={(e) =>
                                  updateStepField("why", e.target.value)
                                }
                              />
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                Args:{" "}
                                {argKeys.length ? argKeys.join(", ") : "-"}
                              </Typography>
                            </Stack>
                          </Box>
                        );
                      })}
                  </Stack>
                ) : (
                  <Typography
                    variant="body2"
                    sx={{
                      color: "text.secondary",
                    }}
                  >
                    AI returned no steps. You can still create the goal.
                  </Typography>
                )}
              </Stack>
            ) : (
              <Alert severity="warning">
                AI draft is unavailable. Update fields above and create
                directly.
              </Alert>
            )}
          </Stack>
        </DialogContent>
        <DialogActions>
          <Button
            onClick={() => {
              if (autopilotMutation.isPending) return;
              setGoalConfirmOpen(false);
              setGoalCreateOpen(true);
            }}
          >
            Back
          </Button>
          <Button
            onClick={() => {
              if (autopilotMutation.isPending) return;
              setGoalConfirmOpen(false);
              resetGoalDraft(true);
            }}
          >
            Cancel
          </Button>
          <Button
            variant="contained"
            startIcon={
              autopilotMutation.isPending ? (
                <CircularProgress size={14} color="inherit" />
              ) : undefined
            }
            disabled={autopilotMutation.isPending || !description.trim()}
            onClick={confirmAutopilotGoal}
          >
            {autopilotMutation.isPending ? "Creating..." : "Confirm & Create"}
          </Button>
        </DialogActions>
      </Dialog>
    </WorkspacePageShell>
  );
}
