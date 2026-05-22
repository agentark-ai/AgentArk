import {
  Alert,
  Box,
  Chip,
  FormControlLabel,
  Stack,
  Switch,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import type { ReactNode } from "react";
import type { SettingsSectionIntroArgs } from "./settingsLayout";

export type SettingsDataLifecycleFormFields = {
  data_lifecycle_cleanup_enabled: boolean;
  data_lifecycle_notifications_cleanup_enabled: boolean;
  data_lifecycle_logs_cleanup_enabled: boolean;
  data_lifecycle_experience_item_retention_days: string;
  data_lifecycle_recall_event_retention_days: string;
  data_lifecycle_recall_test_retention_days: string;
  data_lifecycle_learning_candidate_retention_days: string;
  data_lifecycle_experience_run_retention_days: string;
  data_lifecycle_experience_edge_retention_days: string;
  data_lifecycle_procedural_pattern_retention_days: string;
  data_lifecycle_notifications_retention_days: string;
  data_lifecycle_notification_cleanup_interval_secs: string;
  data_lifecycle_execution_trace_retention_days: string;
  data_lifecycle_execution_run_retention_days: string;
  data_lifecycle_background_session_retention_days: string;
  data_lifecycle_browser_session_retention_days: string;
  data_lifecycle_automation_run_retention_days: string;
  data_lifecycle_execution_proof_retention_days: string;
  data_lifecycle_operational_log_retention_days: string;
  data_lifecycle_security_log_retention_days: string;
  data_lifecycle_approval_log_retention_days: string;
  data_lifecycle_swarm_delegation_retention_days: string;
  data_lifecycle_llm_usage_retention_days: string;
  data_lifecycle_terminal_task_retention_days: string;
  data_lifecycle_message_retention_days: string;
  data_lifecycle_housekeeping_interval_secs: string;
  data_lifecycle_security_cleanup_interval_days: string;
  data_lifecycle_security_cleanup_idle_threshold_secs: string;
};

type SettingsDataLifecyclePanelProps = {
  form: SettingsDataLifecycleFormFields;
  setField: (
    key: keyof SettingsDataLifecycleFormFields,
    value: string | boolean,
  ) => void;
  foreverLifecycleRules: Array<{ label: string; value: string }>;
  foreverLifecycleSummary: string;
  dataCleanupEnabled: boolean;
  notificationsCleanupInputsEnabled: boolean;
  logsCleanupInputsEnabled: boolean;
  renderSettingsSectionIntro: (props: SettingsSectionIntroArgs) => ReactNode;
};

export function SettingsDataLifecyclePanel({
  form,
  setField,
  foreverLifecycleRules,
  foreverLifecycleSummary,
  dataCleanupEnabled,
  notificationsCleanupInputsEnabled,
  logsCleanupInputsEnabled,
  renderSettingsSectionIntro,
}: SettingsDataLifecyclePanelProps) {
  return (
              <Stack spacing={2.5}>
                <Alert
                  severity={
                    foreverLifecycleRules.length > 0 ? "warning" : "info"
                  }
                >
                  <Stack spacing={0.35}>
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      Data cleanup is enabled by default, but every cleanup
                      category can be disabled.
                    </Typography>
                    <Typography
                      variant="body2"
                      sx={{
                        color: "inherit",
                      }}
                    >
                      {foreverLifecycleRules.length > 0
                        ? `Forever is enabled for ${foreverLifecycleSummary}.`
                        : "Set any retention field below to 0 if you intentionally want to keep that data forever."}{" "}
                      Keeping rows forever or far beyond the defaults can
                      increase DB size, slow queries, and make the server feel
                      heavier over time.
                    </Typography>
                  </Stack>
                </Alert>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    <Stack
                      direction="row"
                      spacing={1}
                      useFlexGap
                      sx={{
                        flexWrap: "wrap",
                      }}
                    >
                      <Chip
                        size="small"
                        color={dataCleanupEnabled ? "success" : "default"}
                        label={
                          dataCleanupEnabled
                            ? "Cleanup active"
                            : "Cleanup paused"
                        }
                      />
                      <Chip
                        size="small"
                        variant="outlined"
                        label={
                          form.data_lifecycle_notifications_cleanup_enabled
                            ? "Notifications on"
                            : "Notifications off"
                        }
                      />
                      <Chip
                        size="small"
                        variant="outlined"
                        label={
                          form.data_lifecycle_logs_cleanup_enabled
                            ? "Logs & traces on"
                            : "Logs & traces off"
                        }
                      />
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.data_lifecycle_cleanup_enabled}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable data cleanup"
                    />
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Memory Behavior",
                      description:
                        "Retention controls for durable memory, audit history, staged candidates, and memory checks.",
                    })}
                    <Alert severity="info">
                      Memory is the normal memory surface. These settings
                      only change how long memory records and evidence are
                      retained.
                    </Alert>
                    <Grid2 container spacing={1.5}>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Inactive memory rows (days)"
                          value={
                            form.data_lifecycle_experience_item_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_item_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Active memories are never auto-deleted. 0 keeps inactive rows too."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Memory ledger (days)"
                          value={form.data_lifecycle_recall_event_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_recall_event_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Audit history for memory changes."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Memory checks (days)"
                          value={form.data_lifecycle_recall_test_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_recall_test_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Generated checks for stored memory."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Staged candidates (days)"
                          value={
                            form.data_lifecycle_learning_candidate_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_learning_candidate_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Review queue and rejected candidates."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Experience runs (days)"
                          value={form.data_lifecycle_experience_run_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Session evidence used for learning."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Experience edges (days)"
                          value={
                            form.data_lifecycle_experience_edge_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_experience_edge_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Lineage and supersedes links."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Inactive patterns (days)"
                          value={
                            form.data_lifecycle_procedural_pattern_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_procedural_pattern_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Active and draft learned patterns are never auto-deleted."
                          slotProps={{ htmlInput: { min: 0, step: 1 } }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Notifications",
                      description:
                        "Set retention and cleanup cadence for in-product notifications stored by the system.",
                    })}
                    <FormControlLabel
                      control={
                        <Switch
                          checked={
                            form.data_lifecycle_notifications_cleanup_enabled
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notifications_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable notification cleanup"
                    />
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: notificationsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: notificationsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Retention (days)"
                          value={
                            form.data_lifecycle_notifications_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notifications_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps notifications forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 6 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Cleanup cadence (seconds)"
                          value={
                            form.data_lifecycle_notification_cleanup_interval_secs
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_notification_cleanup_interval_secs",
                              e.target.value,
                            )
                          }
                          helperText="How often stale notifications are purged."
                          slotProps={{
                            htmlInput: { min: 300, step: 60 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Logs & Traces",
                      description:
                        "Retention windows for operational data. Use 0 only when you intentionally want to keep a category forever.",
                    })}
                    <FormControlLabel
                      control={
                        <Switch
                          checked={form.data_lifecycle_logs_cleanup_enabled}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_logs_cleanup_enabled",
                              e.target.checked,
                            )
                          }
                        />
                      }
                      label="Enable logs, traces, task, and message cleanup"
                    />
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: logsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: logsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution traces (days)"
                          value={
                            form.data_lifecycle_execution_trace_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_trace_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all traces."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution runs (days)"
                          value={
                            form.data_lifecycle_execution_run_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all execution runs, checkpoints, and tool attempts."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Background sessions (days)"
                          value={
                            form.data_lifecycle_background_session_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_background_session_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps closed background sessions forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Browser sessions (days)"
                          value={
                            form.data_lifecycle_browser_session_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_browser_session_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps completed and failed browser sessions forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Automation runs (days)"
                          value={
                            form.data_lifecycle_automation_run_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_automation_run_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps automation history forever."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Execution proofs (days)"
                          value={
                            form.data_lifecycle_execution_proof_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_execution_proof_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all proofs."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Operational logs (days)"
                          value={
                            form.data_lifecycle_operational_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_operational_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all operational logs."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security logs (days)"
                          value={
                            form.data_lifecycle_security_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Used by both housekeeping and idle cleanup."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Approval logs (days)"
                          value={
                            form.data_lifecycle_approval_log_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_approval_log_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all approval history."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Delegations (days)"
                          value={
                            form.data_lifecycle_swarm_delegation_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_swarm_delegation_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all delegation records."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="LLM usage (days)"
                          value={form.data_lifecycle_llm_usage_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_llm_usage_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all token/accounting usage."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Completed tasks (days)"
                          value={
                            form.data_lifecycle_terminal_task_retention_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_terminal_task_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="Recurring cron tasks are never purged."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Conversations (days)"
                          value={form.data_lifecycle_message_retention_days}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_message_retention_days",
                              e.target.value,
                            )
                          }
                          helperText="0 keeps all messages and conversation history."
                          slotProps={{
                            htmlInput: { min: 0, step: 1 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>

                <Box className="list-shell">
                  <Stack spacing={2}>
                    {renderSettingsSectionIntro({
                      eyebrow: "Lifecycle",
                      title: "Cleanup Cadence",
                      description:
                        "Configure how often housekeeping runs and when idle security cleanup is allowed to start.",
                    })}
                    <Grid2
                      container
                      spacing={1.5}
                      sx={{
                        opacity: logsCleanupInputsEnabled ? 1 : 0.55,
                        pointerEvents: logsCleanupInputsEnabled
                          ? "auto"
                          : "none",
                        transition: "opacity 0.2s",
                      }}
                    >
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Housekeeping cadence (seconds)"
                          value={form.data_lifecycle_housekeeping_interval_secs}
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_housekeeping_interval_secs",
                              e.target.value,
                            )
                          }
                          helperText="Used for trace, log, task, and message cleanup passes."
                          slotProps={{
                            htmlInput: { min: 300, step: 60 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security cleanup cadence (days)"
                          value={
                            form.data_lifecycle_security_cleanup_interval_days
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_cleanup_interval_days",
                              e.target.value,
                            )
                          }
                          helperText="How often the idle security-log cleanup may run."
                          slotProps={{
                            htmlInput: { min: 1, step: 1 },
                          }}
                        />
                      </Grid2>
                      <Grid2 size={{ xs: 12, md: 4 }}>
                        <TextField
                          fullWidth
                          size="small"
                          type="number"
                          label="Security idle threshold (seconds)"
                          value={
                            form.data_lifecycle_security_cleanup_idle_threshold_secs
                          }
                          onChange={(e) =>
                            setField(
                              "data_lifecycle_security_cleanup_idle_threshold_secs",
                              e.target.value,
                            )
                          }
                          helperText="Server must stay idle this long before the security sweep runs."
                          slotProps={{
                            htmlInput: { min: 60, step: 60 },
                          }}
                        />
                      </Grid2>
                    </Grid2>
                  </Stack>
                </Box>
              </Stack>
  );
}
