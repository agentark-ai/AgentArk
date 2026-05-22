import AutorenewRoundedIcon from "@mui/icons-material/AutorenewRounded";
import CheckCircleRoundedIcon from "@mui/icons-material/CheckCircleRounded";
import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import SettingsRoundedIcon from "@mui/icons-material/SettingsRounded";
import StarRoundedIcon from "@mui/icons-material/StarRounded";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  FormControlLabel,
  Stack,
  Switch,
  TextField,
  Tooltip,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { useUiStore } from "../../store/uiStore";
import { errMessage, str, toBool, type JsonRecord } from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";
import {
  formatDurationClock,
  AUTO_APPROVE_ACTION_OPTIONS,
} from "./settingsPageHelpers";
import { ADVANCED_SENTINEL_SIGNAL_OPTIONS } from "./settingsConstants";

type SettingsAdvancedPanelProps = {
  [key: string]: any;
};

export function SettingsAdvancedPanel({
  restartNotice,
  renderSettingsInlineCard,
  settingsAutonomyQ,
  settingsAutonomyPaused,
  settingsAutonomyModeLabel,
  handleResumeAutonomy,
  setAutonomyPauseDialogOpen,
  settingsAutonomyMutation,
  openRestartDialog,
  restartMutation,
  developerModeEnabled,
  setDeveloperModeEnabledState,
  setError,
  setSuccess,
  settingsSentinelQ,
  settingsSentinel,
  settingsSentinelEnabled,
  setSentinelDisableDialogOpen,
  setSentinelInAppDisableDialogOpen,
  updateSettingsSentinel,
  settingsSentinelMutation,
  settingsEvolutionQ,
  settingsSelfEvolveEnabled,
  handleEnableSelfEvolve,
  setSelfEvolveDisableDialogOpen,
  settingsEvolutionMutation,
  readinessPolicyDraft,
  setReadinessPolicyDraft,
  readinessPolicyToDraft,
  settingsReadinessPolicy,
  submitReadinessPolicyDraft,
  settingsDefaultGuardEnabled,
  updateSettingsEvolution,
  findBlockedAutoApproveEntries,
  parseCsvList,
  sanitizeAutoApproveList,
  form,
  setField,
  apiKeyQ,
  apiKeyRemainingSeconds,
  apiKeyRotated,
  apiKeyRevealed,
  setApiKeyRevealed,
  apiKeyPayload,
  apiKeyIssuedAtUnix,
  apiKeyExpiresAtUnix,
  regenerateApiKeyMutation,
}: SettingsAdvancedPanelProps) {
  return (
              <Stack spacing={2.5}>
                {restartNotice
                  ? renderSettingsInlineCard({
                      eyebrow: "Restarting",
                      title: "AgentArk is coming back online",
                      description: restartNotice.text,
                      tone: "info",
                      action: (
                        <Chip
                          size="small"
                          icon={<AutorenewRoundedIcon />}
                          label={restartNotice.etaLabel}
                          color="info"
                          variant="outlined"
                        />
                      ),
                    })
                  : null}
                {/* -- Warning banner -- */}
                {renderSettingsInlineCard({
                  eyebrow: "Advanced",
                  title: "Use with care",
                  description:
                    "These controls can affect stability, security, or how the product behaves. Change them only if you understand the effect.",
                  fullWidthCopy: true,
                  tone: "warning",
                })}

                {/* -- System Controls group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <SettingsRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">
                        System Controls
                      </div>
                      <div className="adv-group-header-sub">
                        Core runtime and interface options.
                      </div>
                    </div>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.35} sx={{ minWidth: 0 }}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Autonomy Pause
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Pause autonomous background work from this settings
                        surface. Scheduled reminders still fire.
                      </Typography>
                      {settingsAutonomyQ.error ? (
                        <Alert severity="error" sx={{ mt: 0.75 }}>
                          {errMessage(settingsAutonomyQ.error)}
                        </Alert>
                      ) : settingsAutonomyPaused ? (
                        <Alert severity="warning" sx={{ mt: 0.75 }}>
                          Autonomy is paused. Pulse, watchers, background
                          learning, suggestion scans, and proactive
                          optimizations stay paused until you resume it.
                        </Alert>
                      ) : null}
                    </Stack>
                    <Stack
                      direction={{ xs: "column", sm: "row" }}
                      spacing={1}
                      sx={{
                        alignItems: { xs: "stretch", sm: "center" },
                        flexShrink: 0,
                      }}
                    >
                      <Chip
                        size="small"
                        color={settingsAutonomyPaused ? "warning" : "success"}
                        label={settingsAutonomyModeLabel}
                      />
                      <Button
                        size="small"
                        color={settingsAutonomyPaused ? "success" : "warning"}
                        variant="outlined"
                        onClick={() => {
                          if (settingsAutonomyPaused) {
                            void handleResumeAutonomy();
                            return;
                          }
                          setError(null);
                          setSuccess(null);
                          setAutonomyPauseDialogOpen(true);
                        }}
                        disabled={
                          settingsAutonomyQ.isLoading ||
                          !!settingsAutonomyQ.error ||
                          settingsAutonomyMutation.isPending
                        }
                        sx={{ whiteSpace: "nowrap" }}
                      >
                        {settingsAutonomyMutation.isPending
                          ? settingsAutonomyPaused
                            ? "Resuming..."
                            : "Pausing..."
                          : settingsAutonomyPaused
                            ? "Resume autonomy"
                            : "Pause autonomy"}
                      </Button>
                    </Stack>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Restart AgentArk
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Restarts AgentArk to apply runtime and security changes.
                      </Typography>
                    </Stack>
                    <Button
                      size="small"
                      color="warning"
                      variant="outlined"
                      onClick={openRestartDialog}
                      disabled={restartMutation.isPending || !!restartNotice}
                      sx={{ whiteSpace: "nowrap" }}
                    >
                      Restart AgentArk
                    </Button>
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Developer Mode
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Enables raw SKILL.md editing after you save. Keep off for
                        beginner-friendly forms.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={developerModeEnabled}
                          onChange={(e) => {
                            const next = e.target.checked;
                            setDeveloperModeEnabledState(next);
                            setError(null);
                            setSuccess(null);
                          }}
                        />
                      }
                      label={developerModeEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>

                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Guided Tour
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Re-run the onboarding walkthrough to review core
                        features.
                      </Typography>
                    </Stack>
                    <Button
                      size="small"
                      variant="outlined"
                      onClick={() => {
                        try {
                          window.localStorage.setItem(
                            "agentark.tour.completed",
                            "0",
                          );
                        } catch {}
                        const { startTour } = useUiStore.getState();
                        startTour();
                      }}
                      sx={{ whiteSpace: "nowrap" }}
                    >
                      Restart Tour
                    </Button>
                  </div>
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <StarRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">Sentinel</div>
                      <div className="adv-group-header-sub">
                        These switches decide where Sentinel learns from and
                        what kinds of follow-up it can suggest.
                      </div>
                    </div>
                  </div>
                  {settingsSentinelQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsSentinelQ.error)}
                    </Alert>
                  ) : null}
                  {settingsAutonomyPaused ? (
                    <Alert severity="warning" sx={{ mb: 1.5 }}>
                      Sentinel preferences stay saved here, but follow-up
                      scanning is paused until autonomy is active again.
                    </Alert>
                  ) : null}
                  {ADVANCED_SENTINEL_SIGNAL_OPTIONS.map((item) => {
                    const isMainSentinelSwitch = item.key === "enabled";
                    const storedEnabled =
                      settingsSentinel[item.key] == null
                        ? true
                        : toBool(settingsSentinel[item.key]);
                    const checked = isMainSentinelSwitch
                      ? storedEnabled
                      : settingsSentinelEnabled && storedEnabled;
                    return (
                      <div className="adv-row" key={item.key}>
                        <Stack spacing={0.2}>
                          <Typography
                            variant="body2"
                            sx={{ fontWeight: 600 }}
                          >
                            {item.label}
                          </Typography>
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            {item.description}
                          </Typography>
                        </Stack>
                        <FormControlLabel
                          control={
                            <Switch
                              checked={checked}
                              onChange={(event) => {
                                if (
                                  item.key === "enabled" &&
                                  !event.target.checked
                                ) {
                                  setError(null);
                                  setSuccess(null);
                                  setSentinelDisableDialogOpen(true);
                                  return;
                                }
                                if (
                                  item.key === "watch_in_app" &&
                                  !event.target.checked
                                ) {
                                  setError(null);
                                  setSuccess(null);
                                  setSentinelInAppDisableDialogOpen(true);
                                  return;
                                }
                                const nextChecked = event.target.checked;
                                const payload: JsonRecord =
                                  item.key === "enabled" && nextChecked
                                    ? {
                                        enabled: true,
                                        watch_in_app: true,
                                        watch_connected_services: true,
                                        infer_new_automations: true,
                                      }
                                    : ({
                                        [item.key]: nextChecked,
                                      } as JsonRecord);
                                void updateSettingsSentinel(
                                  payload,
                                  item.key === "enabled" && nextChecked
                                    ? "Sentinel and all signal switches are on."
                                    : nextChecked
                                      ? item.enabledMessage
                                      : item.disabledMessage,
                                );
                              }}
                              disabled={
                                settingsSentinelQ.isLoading ||
                                !!settingsSentinelQ.error ||
                                settingsSentinelMutation.isPending ||
                                (!isMainSentinelSwitch &&
                                  !settingsSentinelEnabled)
                              }
                            />
                          }
                          label={checked ? "On" : "Off"}
                          sx={{ mr: 0 }}
                        />
                      </div>
                    );
                  })}
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <AutorenewRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">Evolve</div>
                      <div className="adv-group-header-sub">
                        Controls whether AgentArk learns from completed work and
                        tests reviewed improvements in the background.
                      </div>
                    </div>
                  </div>
                  {settingsEvolutionQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsEvolutionQ.error)}
                    </Alert>
                  ) : null}
                  {settingsAutonomyPaused ? (
                    <Alert severity="warning" sx={{ mb: 1.5 }}>
                      Self-evolve can stay on, but its background passes will
                      remain paused until autonomy is active again.
                    </Alert>
                  ) : null}
                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Self-evolve
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        Controls heuristic reflection, consolidation, candidate
                        generation, and active canary experiments.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={settingsSelfEvolveEnabled}
                          onChange={(event) => {
                            if (event.target.checked) {
                              void handleEnableSelfEvolve();
                              return;
                            }
                            setError(null);
                            setSuccess(null);
                            setSelfEvolveDisableDialogOpen(true);
                          }}
                          disabled={
                            settingsEvolutionQ.isLoading ||
                            !!settingsEvolutionQ.error ||
                            settingsEvolutionMutation.isPending
                          }
                        />
                      }
                      label={settingsSelfEvolveEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>
                  <Accordion disableGutters sx={{ mt: 1 }}>
                    <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                      <Stack spacing={0.2}>
                        <Typography variant="body2" sx={{ fontWeight: 600 }}>
                          Readiness gates
                        </Typography>
                        <Typography variant="caption" sx={{ color: "text.secondary" }}>
                          Human review and auto-run stay separate. Auto-run
                          requires stronger repeated evidence.
                        </Typography>
                      </Stack>
                    </AccordionSummary>
                    <AccordionDetails>
                      <Alert severity="info" sx={{ borderRadius: 1, mb: 1.25 }}>
                        For normal use, leave these defaults alone. Lower values
                        make Evolve suggest changes sooner; higher values make
                        it wait for more proof.
                      </Alert>
                      <Grid2 container spacing={1.25}>
                        {[
                          ["min_review_samples", "Review samples", "Runs needed before a suggestion can be approved"],
                          ["min_auto_samples", "Auto-run samples", "Runs needed before automatic use is even considered"],
                          ["min_review_success_rate_pct", "Review success %", "Minimum success rate for review"],
                          ["min_auto_success_rate_pct", "Auto-run success %", "Minimum success rate for auto-run"],
                          ["max_review_correction_rate_pct", "Review correction %", "Maximum correction rate for review"],
                          ["max_auto_correction_rate_pct", "Auto-run correction %", "Maximum correction rate for auto-run"],
                          ["min_candidate_review_confidence_pct", "Candidate confidence %", "Minimum confidence before review"],
                          ["max_review_trust_score", "Review risk score", "Highest trust risk allowed for review"],
                          ["max_auto_trust_score", "Auto-run risk score", "Highest trust risk allowed for auto-run"],
                        ].map(([key, label, helper]) => (
                          <Grid2 key={key} size={{ xs: 12, sm: 6, lg: 4 }}>
                            <TextField
                              fullWidth
                              size="small"
                              type="number"
                              label={label}
                              value={readinessPolicyDraft[key] ?? ""}
                              onChange={(event) =>
                                setReadinessPolicyDraft((draft: Record<string, string>) => ({
                                  ...draft,
                                  [key]: event.target.value,
                                }))
                              }
                              helperText={helper}
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                !!settingsEvolutionQ.error ||
                                settingsEvolutionMutation.isPending
                              }
                            />
                          </Grid2>
                        ))}
                        <Grid2 size={{ xs: 12 }}>
                          <Stack
                            direction={{ xs: "column", sm: "row" }}
                            spacing={1}
                            sx={{ justifyContent: "flex-end" }}
                          >
                            <Button
                              size="small"
                              color="inherit"
                              onClick={() =>
                                setReadinessPolicyDraft(
                                  readinessPolicyToDraft(settingsReadinessPolicy),
                                )
                              }
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                settingsEvolutionMutation.isPending
                              }
                            >
                              Reset
                            </Button>
                            <Button
                              size="small"
                              variant="contained"
                              onClick={() => void submitReadinessPolicyDraft()}
                              disabled={
                                settingsEvolutionQ.isLoading ||
                                !!settingsEvolutionQ.error ||
                                settingsEvolutionMutation.isPending
                              }
                            >
                              Save gates
                            </Button>
                          </Stack>
                        </Grid2>
                      </Grid2>
                    </AccordionDetails>
                  </Accordion>
                </Box>

                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <CheckCircleRoundedIcon
                        sx={{ fontSize: 16, color: "var(--ui-rgba-244-245-247-820)" }}
                      />
                    </div>
                    <div>
                      <div className="adv-group-header-title">
                        App Deploy Defaults
                      </div>
                      <div className="adv-group-header-sub">
                        Deployment defaults that should stay separate from
                        Sentinel and Evolve.
                      </div>
                    </div>
                  </div>
                  {settingsEvolutionQ.error ? (
                    <Alert severity="error" sx={{ mb: 1.5 }}>
                      {errMessage(settingsEvolutionQ.error)}
                    </Alert>
                  ) : null}
                  <div className="adv-row">
                    <Stack spacing={0.2}>
                      <Typography variant="body2" sx={{ fontWeight: 600 }}>
                        Default app access guard
                      </Typography>
                      <Typography
                        variant="caption"
                        sx={{
                          color: "text.secondary",
                        }}
                      >
                        New app deploy and public-link flows start with the
                        access guard on unless a request explicitly overrides
                        it.
                      </Typography>
                    </Stack>
                    <FormControlLabel
                      control={
                        <Switch
                          checked={settingsDefaultGuardEnabled}
                          onChange={(event) =>
                            void updateSettingsEvolution(
                              {
                                deploy_guard_default: event.target.checked,
                              },
                              event.target.checked
                                ? "New app deploys will start with the access guard on by default."
                                : "New app deploys will leave the access guard off by default.",
                            )
                          }
                          disabled={
                            settingsEvolutionQ.isLoading ||
                            !!settingsEvolutionQ.error ||
                            settingsEvolutionMutation.isPending
                          }
                        />
                      }
                      label={settingsDefaultGuardEnabled ? "On" : "Off"}
                      sx={{ mr: 0 }}
                    />
                  </div>
                </Box>

                {/* -- Permissions group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <span style={{ fontSize: 15 }}>&#128274;</span>
                    </div>
                    <div>
                      <div className="adv-group-header-title">Permissions</div>
                      <div className="adv-group-header-sub">
                        Action approval and auto-approve settings.
                      </div>
                    </div>
                  </div>

                  {/* Auto-Approve Skills */}
                  <Typography variant="body2" sx={{ fontWeight: 600, mb: 0.5 }}>
                    Auto-Approve Skills
                  </Typography>
                  <Typography
                    variant="caption"
                    sx={{
                      color: "text.secondary",
                      display: "block",
                      mb: 1.5,
                    }}
                  >
                    Select action-name overrides that can run without a separate
                    approval prompt. Dangerous actions stay approval-gated even
                    if typed manually.
                  </Typography>
                  {(() => {
                    const blockedEntries = findBlockedAutoApproveEntries(
                      form.auto_approve_csv,
                    );
                    const set = new Set(
                      sanitizeAutoApproveList(
                        parseCsvList(form.auto_approve_csv),
                      ),
                    );
                    const update = (name: string, checked: boolean) => {
                      const next = new Set(set);
                      if (checked) next.add(name);
                      else next.delete(name);
                      setField(
                        "auto_approve_csv",
                        sanitizeAutoApproveList(Array.from(next).sort()).join(
                          ", ",
                        ),
                      );
                    };
                    return (
                      <>
                        {blockedEntries.length > 0 ? (
                          <Alert severity="warning" sx={{ mb: 1.5 }}>
                            These actions always require approval and will be
                            ignored here:{" "}
                            {blockedEntries
                              .map((name: string) => `\`${name}\``)
                              .join(", ")}
                            .
                          </Alert>
                        ) : null}
                        <Grid2 container spacing={1}>
                          {AUTO_APPROVE_ACTION_OPTIONS.map((name: string) => {
                            const active = set.has(name);
                            return (
                              <Grid2 key={name} size={{ xs: 6, md: 4, lg: 3 }}>
                                <div
                                  className={`adv-skill-pill${active ? " active" : ""}`}
                                  onClick={() => update(name, !active)}
                                >
                                  <Typography
                                    variant="caption"
                                    sx={{
                                      fontFamily: "'JetBrains Mono', monospace",
                                      fontSize: "0.7rem",
                                      letterSpacing: 0,
                                    }}
                                  >
                                    {name}
                                  </Typography>
                                  <Switch
                                    size="small"
                                    checked={active}
                                    onChange={(e) =>
                                      update(name, e.target.checked)
                                    }
                                  />
                                </div>
                              </Grid2>
                            );
                          })}
                        </Grid2>
                        <TextField
                          label="Custom (CSV)"
                          value={form.auto_approve_csv}
                          onChange={(e) =>
                            setField("auto_approve_csv", e.target.value)
                          }
                          fullWidth
                          size="small"
                          placeholder="comma separated action names"
                          helperText="Always blocked here: shell, file_write, code_execute, lan_discover, gmail_send, and similar sensitive actions."
                          sx={{ mt: 1.5 }}
                        />
                      </>
                    );
                  })()}
                </Box>

                {/* -- API Access group -- */}
                <Box className="adv-group">
                  <div className="adv-group-header">
                    <div className="adv-group-header-icon">
                      <span style={{ fontSize: 15 }}>&#128273;</span>
                    </div>
                    <div>
                      <div className="adv-group-header-title">API Access</div>
                      <div className="adv-group-header-sub">
                        HTTP API key management.
                      </div>
                    </div>
                  </div>

                  {apiKeyQ.isLoading ? (
                    <Typography
                      variant="body2"
                      sx={{
                        color: "text.secondary",
                      }}
                    >
                      Loading API key...
                    </Typography>
                  ) : apiKeyQ.error ? (
                    <Alert severity="error">{errMessage(apiKeyQ.error)}</Alert>
                  ) : (
                    <Stack spacing={1.5}>
                      <Stack
                        direction="row"
                        spacing={2}
                        sx={{
                          alignItems: "center",
                          flexWrap: "wrap",
                        }}
                      >
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                            flex: "1 1 auto",
                          }}
                        >
                          Used as{" "}
                          <code
                            style={{
                              background: "var(--ui-rgba-255-255-255-060)",
                              padding: "1px 5px",
                              borderRadius: 2,
                              fontSize: "0.72rem",
                              color: "var(--ui-rgba-244-245-247-900)",
                            }}
                          >
                            Authorization: Bearer &lt;key&gt;
                          </code>{" "}
                          for all HTTP requests.
                        </Typography>
                        <Chip
                          size="small"
                          color={
                            apiKeyRemainingSeconds > 0 ? "info" : "warning"
                          }
                          label={`Rotates in ${formatDurationClock(apiKeyRemainingSeconds)}`}
                        />
                      </Stack>
                      {apiKeyRotated ? (
                        <Chip
                          size="small"
                          color="success"
                          label="API key rotated automatically"
                        />
                      ) : null}
                      <TextField
                        label="Key"
                        value={
                          apiKeyRevealed
                            ? str(apiKeyPayload.key, "")
                            : str(apiKeyPayload.masked, "")
                        }
                        fullWidth
                        size="small"
                        slotProps={{
                          input: {
                            readOnly: true,
                            sx: {
                              fontFamily:
                                "'JetBrains Mono', 'Fira Code', monospace",
                              fontSize: "0.78rem",
                              letterSpacing: 0,
                            },
                          },
                        }}
                      />
                      {apiKeyIssuedAtUnix > 0
                        ? (() => {
                            const { label: issuedLabel, tip: issuedTip } =
                              humanTs(
                                new Date(
                                  apiKeyIssuedAtUnix * 1000,
                                ).toISOString(),
                              );
                            return (
                              <Tooltip title={issuedTip} placement="top">
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                    cursor: "default",
                                  }}
                                >
                                  Issued {issuedLabel}
                                </Typography>
                              </Tooltip>
                            );
                          })()
                        : null}
                      {apiKeyExpiresAtUnix > 0
                        ? (() => {
                            const { label: expiresLabel, tip: expiresTip } =
                              humanTs(
                                new Date(
                                  apiKeyExpiresAtUnix * 1000,
                                ).toISOString(),
                              );
                            return (
                              <Tooltip title={expiresTip} placement="top">
                                <Typography
                                  variant="caption"
                                  sx={{
                                    color: "text.secondary",
                                    cursor: "default",
                                  }}
                                >
                                  Expires {expiresLabel}
                                </Typography>
                              </Tooltip>
                            );
                          })()
                        : null}
                      <Stack direction="row" spacing={1}>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={() => setApiKeyRevealed((v: boolean) => !v)}
                        >
                          {apiKeyRevealed ? "Hide" : "Reveal"}
                        </Button>
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={async () => {
                            const key = str(apiKeyPayload.key, "");
                            if (!key) return;
                            await navigator.clipboard.writeText(key);
                            setSuccess("API key copied.");
                          }}
                          disabled={!str(apiKeyPayload.key, "").trim()}
                        >
                          Copy
                        </Button>
                        <Button
                          size="small"
                          color="warning"
                          variant="outlined"
                          onClick={async () => {
                            const ok = window.confirm(
                              "Regenerate API key? Old key will stop working.",
                            );
                            if (!ok) return;
                            setError(null);
                            setSuccess(null);
                            try {
                              await regenerateApiKeyMutation.mutateAsync();
                              setApiKeyRevealed(true);
                              setSuccess("API key regenerated.");
                            } catch (e) {
                              setError(errMessage(e));
                            }
                          }}
                          disabled={regenerateApiKeyMutation.isPending}
                        >
                          Regenerate
                        </Button>
                      </Stack>
                    </Stack>
                  )}
                </Box>
              </Stack>
  );
}
