import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import LockRoundedIcon from "@mui/icons-material/LockRounded";
import ShieldRoundedIcon from "@mui/icons-material/ShieldRounded";
import VpnKeyRoundedIcon from "@mui/icons-material/VpnKeyRounded";
import PublicRoundedIcon from "@mui/icons-material/PublicRounded";
import InventoryRoundedIcon from "@mui/icons-material/Inventory2Rounded";
import VisibilityOffRoundedIcon from "@mui/icons-material/VisibilityOffRounded";
import {
  Accordion,
  AccordionDetails,
  AccordionSummary,
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  FormControlLabel,
  MenuItem,
  Stack,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableContainer,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from "@mui/material";
import Grid2 from "@mui/material/Grid";
import { asRecord, errMessage, num, str, toBool } from "./pageHelpers";
import { humanTs } from "./workspaceUiBits";
import {
  getTunnelPanelWarning,
  getTunnelStartButtonLabel,
  getTunnelStopButtonLabel,
  getTunnelUrlFieldLabel,
} from "../../lib/tunnelAccess";
import {
  tunnelCheckAlertSeverity,
  tunnelCheckChipColor,
  tunnelCheckLabel,
} from "./settingsPageHelpers";

type SettingsSecurityPanelProps = {
  [key: string]: any;
};

export function SettingsSecurityPanel({
  renderSettingsSectionIntro,
  securityStatusQ,
  hasCustomMasterPassword,
  passwordMutationPending,
  openPasswordDialog,
  abuseReviews,
  abuseReviewsQ,
  decideAbuseReviewMutation,
  showInternalServiceSection,
  internalServiceDescription,
  internalServiceRotationSupported,
  internalServiceTokens,
  openRotateInternalCredentialsDialog,
  rotateInternalServiceTokensMutation,
  restartNotice,
  tunnelQ,
  tunnelProvidersQ,
  tunnel,
  tunnelProvidersPayload,
  selectedTunnelMeta,
  selectedTunnelStoredSecretFields,
  showTunnelAdvanced,
  setShowTunnelAdvanced,
  tunnelDraftValues,
  setTunnelDraftValues,
  tunnelSelectedProviderId,
  tunnelPanelNotice,
  serverSelectedTunnelProviderId,
  tunnelSummaryTone,
  tunnelStateLabel,
  tunnelAccessLabel,
  tunnelPrimaryText,
  tunnelPrimaryDetail,
  tunnelProviderOptions,
  basicTunnelConfigFields,
  advancedTunnelConfigFields,
  selectedTunnelAvailable,
  tunnelSetupChecks,
  renderSettingsInlineCard,
  tunnelSaveMutation,
  tunnelTestMutation,
  tunnelStartMutation,
  tunnelStopMutation,
  handleTunnelStart,
  handleTunnelStop,
  syncTunnelDraftFromPayload,
  handleTunnelProviderSave,
  handleTunnelProviderTest,
  vaultSummaryText,
  vaultSecrets,
  vaultPassword,
  setVaultPassword,
  vaultSecretsQ,
  queryClient,
  openVaultEditor,
  deleteVaultSecretMutation,
  resolveVaultPasswordForSensitiveOps,
  form,
  setField,
  setError,
  setSuccess,
}: SettingsSecurityPanelProps) {
  return (
              <Stack spacing={2}>
                {renderSettingsInlineCard({
                  eyebrow: "Security",
                  title: "Review before changing",
                  description:
                    "These controls govern who can sign in, what credentials live on this instance, and how the public surface is exposed. Changes take effect immediately for new sessions.",
                  fullWidthCopy: true,
                  tone: "warning",
                })}
                <Box
                  sx={{
                    display: "grid",
                    gap: 2,
                    alignItems: "start",
                    gridTemplateColumns: "minmax(0, 1fr)",
                  }}
                >
                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "security" }}
                    >
                      <Stack spacing={1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Security & Master Password",
                          description:
                            "Protect operator access, control remote sign-in, and manage the primary instance password.",
                          icon: <LockRoundedIcon fontSize="small" />,
                        })}
                        {securityStatusQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading security status...
                          </Typography>
                        ) : securityStatusQ.error ? (
                          <Alert severity="error">
                            {errMessage(securityStatusQ.error)}
                          </Alert>
                        ) : hasCustomMasterPassword ? (
                          <Stack spacing={1.1}>
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={1}
                            >
                              <Button
                                variant="contained"
                                size="large"
                                onClick={() => openPasswordDialog("change")}
                                disabled={passwordMutationPending}
                              >
                                Change Password
                              </Button>
                              <Button
                                color="error"
                                variant="outlined"
                                size="large"
                                onClick={() => openPasswordDialog("remove")}
                                disabled={passwordMutationPending}
                              >
                                Remove Password
                              </Button>
                            </Stack>
                          </Stack>
                        ) : null}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "abuse" }}
                    >
                      <Stack spacing={1.1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Abuse Review",
                          description:
                            "Resume or pause sources that repeatedly tripped the inbound semantic guard.",
                          icon: <ShieldRoundedIcon fontSize="small" />,
                          action: (
                            <Chip
                              size="small"
                              color={abuseReviews.length > 0 ? "warning" : "success"}
                              variant="outlined"
                              label={`${abuseReviews.length} waiting`}
                            />
                          ),
                        })}
                        {abuseReviewsQ.isLoading ? (
                          <Typography variant="body2" sx={{ color: "text.secondary" }}>
                            Loading review queue...
                          </Typography>
                        ) : abuseReviewsQ.error ? (
                          <Alert severity="error">{errMessage(abuseReviewsQ.error)}</Alert>
                        ) : abuseReviews.length === 0 ? (
                          <Alert severity="success" sx={{ py: 0.25 }}>
                            No sources are paused or waiting for review.
                          </Alert>
                        ) : (
                          <TableContainer className="table-shell" sx={{ width: "100%", overflowX: "auto" }}>
                            <Table size="small" sx={{ tableLayout: "fixed", width: "100%" }}>
                              <TableHead>
                                <TableRow>
                                  <TableCell sx={{ width: "22%" }}>Status</TableCell>
                                  <TableCell sx={{ width: "24%" }}>Source</TableCell>
                                  <TableCell sx={{ width: "16%" }}>Trips</TableCell>
                                  <TableCell sx={{ width: "18%" }}>Updated</TableCell>
                                  <TableCell sx={{ width: "20%" }} align="right">
                                    Decision
                                  </TableCell>
                                </TableRow>
                              </TableHead>
                              <TableBody>
                                {abuseReviews.map((row: any, index: number) => {
                                  const sourceKeyHash = str(row.source_key_hash, "");
                                  const status = str(row.status, "");
                                  const source = str(row.channel_id, "channel");
                                  const identity = str(row.user_identity, "").trim();
                                  const updatedAt = humanTs(str(row.last_updated, ""));
                                  const pending = decideAbuseReviewMutation.isPending;
                                  return (
                                    <TableRow key={sourceKeyHash || `abuse-review-${index}`}>
                                      <TableCell>
                                        <Chip
                                          size="small"
                                          color={status === "paused" ? "error" : "warning"}
                                          variant="outlined"
                                          label={status === "paused" ? "Paused" : "Pending review"}
                                        />
                                      </TableCell>
                                      <TableCell sx={{ overflow: "hidden" }}>
                                        <Typography variant="body2" noWrap title={identity ? `${source} / ${identity}` : source}>
                                          {identity ? `${source} / ${identity}` : source}
                                        </Typography>
                                        <Typography variant="caption" sx={{ color: "text.secondary" }} noWrap title={sourceKeyHash}>
                                          {sourceKeyHash.slice(0, 12)}
                                        </Typography>
                                      </TableCell>
                                      <TableCell>{num(row.trip_count, 0)}</TableCell>
                                      <TableCell>
                                        <Typography variant="caption" title={updatedAt.tip}>
                                          {updatedAt.label}
                                        </Typography>
                                      </TableCell>
                                      <TableCell align="right">
                                        <Stack direction="row" spacing={0.75} sx={{ justifyContent: "flex-end" }}>
                                          <Button
                                            size="small"
                                            variant="outlined"
                                            disabled={pending || !sourceKeyHash}
                                            onClick={() =>
                                              decideAbuseReviewMutation.mutate({
                                                sourceKeyHash,
                                                decision: "reject",
                                              })
                                            }
                                          >
                                            Pause
                                          </Button>
                                          <Button
                                            size="small"
                                            variant="contained"
                                            disabled={pending || !sourceKeyHash}
                                            onClick={() =>
                                              decideAbuseReviewMutation.mutate({
                                                sourceKeyHash,
                                                decision: "approve",
                                              })
                                            }
                                          >
                                            Resume
                                          </Button>
                                        </Stack>
                                      </TableCell>
                                    </TableRow>
                                  );
                                })}
                              </TableBody>
                            </Table>
                          </TableContainer>
                        )}
                      </Stack>
                    </Box>

                    {showInternalServiceSection ? (
                      <Box
                        className="list-shell"
                        sx={{ minHeight: 0, gridArea: "internal" }}
                      >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Internal Service Credentials",
                          description: internalServiceDescription,
                          icon: <VpnKeyRoundedIcon fontSize="small" />,
                          action: internalServiceRotationSupported ? (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{ flexWrap: "wrap" }}
                            >
                              <Chip size="small" label="Manual rotation" />
                              <Chip
                                size="small"
                                variant="outlined"
                                label="Restart required"
                              />
                            </Stack>
                          ) : undefined,
                        })}
                        {securityStatusQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            Loading internal credential status...
                          </Typography>
                        ) : securityStatusQ.error ? (
                          <Alert severity="error">
                            {errMessage(securityStatusQ.error)}
                          </Alert>
                        ) : internalServiceTokens.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary" }}
                          >
                            Internal executor and workspace credentials are not
                            available on this runtime.
                          </Typography>
                        ) : (
                          <Stack spacing={1.1}>
                            <Stack
                              divider={
                                <Divider
                                  flexItem
                                  sx={{ borderColor: "divider" }}
                                />
                              }
                              spacing={0}
                            >
                              {internalServiceTokens.map((row: any, index: number) => {
                                const item = asRecord(row);
                                const updatedAt = humanTs(
                                  str(item.updated_at, ""),
                                );
                                const managedByEnv = toBool(
                                  item.managed_by_env,
                                );
                                const configured = toBool(item.configured);
                                return (
                                  <Stack
                                    key={str(item.id, `token-${index}`)}
                                    direction={{ xs: "column", sm: "row" }}
                                    spacing={1}
                                    sx={{ py: 1 }}
                                  >
                                    <Stack
                                      spacing={0.35}
                                      sx={{ minWidth: 0, flex: 1 }}
                                    >
                                      <Typography
                                        variant="body2"
                                        sx={{ fontWeight: 600 }}
                                      >
                                        {str(item.label, "Internal service")}
                                      </Typography>
                                      <Typography
                                        variant="caption"
                                        sx={{ color: "text.secondary" }}
                                      >
                                        {managedByEnv
                                          ? `Managed by ${str(item.env_var, "environment configuration")}`
                                          : "Stored in the AgentArk config volume"}
                                      </Typography>
                                    </Stack>
                                    <Stack
                                      direction="row"
                                      spacing={0.75}
                                      useFlexGap
                                      sx={{
                                        flexWrap: "wrap",
                                        alignItems: "center",
                                      }}
                                    >
                                      <Chip
                                        size="small"
                                        color={
                                          configured ? "success" : "warning"
                                        }
                                        label={
                                          configured ? "Configured" : "Missing"
                                        }
                                      />
                                      <Chip
                                        size="small"
                                        variant="outlined"
                                        label={
                                          managedByEnv
                                            ? "Env managed"
                                            : `Updated ${updatedAt.label}`
                                        }
                                        title={
                                          managedByEnv
                                            ? undefined
                                            : updatedAt.tip
                                        }
                                      />
                                    </Stack>
                                  </Stack>
                                );
                              })}
                            </Stack>
                            {internalServiceRotationSupported ? (
                              <Alert severity="info">
                                Rotation rewrites both credentials together,
                                then restarts control, executor, and workspace
                                immediately. Active work can be interrupted
                                while the stack comes back.
                              </Alert>
                            ) : null}
                            {internalServiceRotationSupported ? (
                              <Stack
                                direction={{ xs: "column", sm: "row" }}
                                spacing={1}
                                useFlexGap
                                sx={{ flexWrap: "wrap" }}
                              >
                                <Button
                                  variant="outlined"
                                  color="warning"
                                  size="large"
                                  disabled={
                                    rotateInternalServiceTokensMutation.isPending ||
                                    !!restartNotice
                                  }
                                  onClick={openRotateInternalCredentialsDialog}
                                >
                                  {rotateInternalServiceTokensMutation.isPending
                                    ? "Rotating..."
                                    : "Rotate Internal Credentials"}
                                </Button>
                              </Stack>
                            ) : null}
                          </Stack>
                        )}
                      </Stack>
                      </Box>
                    ) : null}

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "remote" }}
                    >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Remote Access",
                          description:
                            "Only expose remote sign-in when you need it, and keep the access method and posture visible.",
                          icon: <PublicRoundedIcon fontSize="small" />,
                          action: (
                            <Stack
                              direction="row"
                              spacing={0.75}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                              }}
                            >
                              <Chip
                                size="small"
                                color={tunnelSummaryTone}
                                label={tunnelStateLabel}
                              />
                              <Chip
                                size="small"
                                variant="outlined"
                                label={tunnelAccessLabel}
                              />
                            </Stack>
                          ),
                        })}
                        {tunnelQ.isLoading || tunnelProvidersQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading tunnel settings...
                          </Typography>
                        ) : tunnelQ.error || tunnelProvidersQ.error ? (
                          <Alert severity="error">
                            {errMessage(
                              tunnelQ.error || tunnelProvidersQ.error,
                            )}
                          </Alert>
                        ) : (
                          <Stack spacing={1.1}>
                            <Alert
                              severity={tunnelSummaryTone}
                              sx={{
                                py: 0.25,
                                "& .MuiAlert-message": { width: "100%" },
                              }}
                            >
                              <Stack spacing={0.35}>
                                <Typography
                                  variant="body2"
                                  sx={{ fontWeight: 600 }}
                                >
                                  {tunnelPrimaryText}
                                </Typography>
                                <Typography
                                  variant="body2"
                                  sx={{
                                    color: "inherit",
                                  }}
                                >
                                  {tunnelPrimaryDetail}
                                </Typography>
                              </Stack>
                            </Alert>
                            <TextField
                              label="Access method"
                              select
                              size="small"
                              fullWidth
                              value={
                                tunnelSelectedProviderId ||
                                serverSelectedTunnelProviderId
                              }
                              onChange={(e) => {
                                const next = e.target.value;
                                syncTunnelDraftFromPayload(
                                  tunnelProvidersPayload,
                                  next,
                                );
                              }}
                            >
                              {tunnelProviderOptions.map((provider: any) => {
                                const id = str(provider.id, "");
                                const label = str(
                                  provider.label,
                                  id || "Provider",
                                );
                                const available = toBool(provider.available);
                                return (
                                  <MenuItem key={id} value={id}>
                                    {available
                                      ? label
                                      : `${label} (not available)`}
                                  </MenuItem>
                                );
                              })}
                            </TextField>
                            {basicTunnelConfigFields.map((field: any) => {
                              const key = str(field.key, "");
                              const inputType = str(field.input_type, "text");
                              const options = Array.isArray(field.options)
                                ? field.options.filter(
                                    (value: unknown): value is string =>
                                      typeof value === "string",
                                  )
                                : [];
                              const value = tunnelDraftValues[key] ?? "";
                              const storedSecret =
                                inputType === "password" &&
                                selectedTunnelStoredSecretFields.includes(key);
                              const helperText =
                                inputType === "password" &&
                                storedSecret &&
                                !value.trim()
                                  ? "A value is already saved. Enter a new value only if you want to replace it."
                                  : undefined;
                              return (
                                <TextField
                                  key={key}
                                  label={str(field.label, key || "Field")}
                                  value={value}
                                  onChange={(e) =>
                                    setTunnelDraftValues((prev: any) => ({
                                      ...prev,
                                      [key]: e.target.value,
                                    }))
                                  }
                                  fullWidth
                                  size="small"
                                  required={toBool(field.required)}
                                  placeholder={
                                    str(field.placeholder, "") || undefined
                                  }
                                  type={
                                    inputType === "password"
                                      ? "password"
                                      : "text"
                                  }
                                  multiline={inputType === "textarea"}
                                  minRows={
                                    inputType === "textarea" ? 3 : undefined
                                  }
                                  select={inputType === "select"}
                                  helperText={helperText}
                                >
                                  {inputType === "select"
                                    ? options.map((option: string) => (
                                        <MenuItem key={option} value={option}>
                                          {option}
                                        </MenuItem>
                                      ))
                                    : null}
                                </TextField>
                              );
                            })}
                            {tunnelPanelNotice ? (
                              <Alert severity={tunnelPanelNotice.severity}>
                                {tunnelPanelNotice.text}
                              </Alert>
                            ) : null}
                            {str(tunnel.error, "").trim() ? (
                              <Alert severity="error">
                                {str(tunnel.error)}
                              </Alert>
                            ) : null}
                            {tunnelSetupChecks.length > 0
                              ? renderSettingsInlineCard({
                                  eyebrow: "Remote access",
                                  title: "Before remote access can start",
                                  description:
                                    "This checklist shows what is still missing, with the exact fix for each step.",
                                  tone: "info",
                                  children: (
                                    <Stack spacing={1}>
                                      {tunnelSetupChecks.map((rawCheck: any, index: number) => {
                                          const check = asRecord(rawCheck);
                                          const status = str(
                                            check.status,
                                            "info",
                                          );
                                          const detail = str(check.detail, "");
                                          const remediation = str(
                                            check.remediation,
                                            "",
                                          ).trim();
                                          return (
                                            <Alert
                                              key={`${str(check.id, `check-${index}`)}-${index}`}
                                              severity={tunnelCheckAlertSeverity(
                                                status,
                                              )}
                                              sx={{
                                                py: 0.25,
                                                "& .MuiAlert-message": {
                                                  width: "100%",
                                                },
                                              }}
                                            >
                                              <Stack spacing={0.45}>
                                                <Stack
                                                  direction="row"
                                                  spacing={0.75}
                                                  useFlexGap
                                                  sx={{
                                                    alignItems: "center",
                                                    flexWrap: "wrap",
                                                  }}
                                                >
                                                  <Chip
                                                    size="small"
                                                    color={tunnelCheckChipColor(
                                                      status,
                                                    )}
                                                    label={tunnelCheckLabel(
                                                      status,
                                                    )}
                                                  />
                                                  <Typography
                                                    variant="body2"
                                                    sx={{ fontWeight: 600 }}
                                                  >
                                                    {str(
                                                      check.label,
                                                      "Setup step",
                                                    )}
                                                  </Typography>
                                                </Stack>
                                                {detail ? (
                                                  <Typography
                                                    variant="body2"
                                                    sx={{
                                                      color: "inherit",
                                                    }}
                                                  >
                                                    {detail}
                                                  </Typography>
                                                ) : null}
                                                {remediation ? (
                                                  <Typography
                                                    variant="caption"
                                                    sx={{
                                                      color: "inherit",
                                                      opacity: 0.85,
                                                    }}
                                                  >
                                                    {remediation}
                                                  </Typography>
                                                ) : null}
                                              </Stack>
                                            </Alert>
                                          );
                                        },
                                      )}
                                    </Stack>
                                  ),
                                })
                              : null}
                            {str(tunnel.url, "").trim() ? (
                              <TextField
                                label={getTunnelUrlFieldLabel(
                                  selectedTunnelMeta,
                                )}
                                value={str(tunnel.url)}
                                fullWidth
                                size="small"
                                slotProps={{
                                  input: { readOnly: true },
                                }}
                              />
                            ) : null}
                            <Stack
                              direction={{ xs: "column", sm: "row" }}
                              spacing={1}
                              useFlexGap
                              sx={{
                                flexWrap: "wrap",
                              }}
                            >
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={handleTunnelProviderSave}
                                disabled={tunnelSaveMutation.isPending}
                              >
                                {tunnelSaveMutation.isPending
                                  ? "Saving..."
                                  : "Save"}
                              </Button>
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={handleTunnelProviderTest}
                                disabled={
                                  tunnelSaveMutation.isPending ||
                                  tunnelTestMutation.isPending
                                }
                              >
                                {tunnelTestMutation.isPending
                                  ? "Checking..."
                                  : "Check setup"}
                              </Button>
                              <Button
                                size="small"
                                variant="contained"
                                onClick={handleTunnelStart}
                                disabled={
                                  tunnelSaveMutation.isPending ||
                                  tunnelStartMutation.isPending ||
                                  toBool(tunnel.active) ||
                                  !selectedTunnelAvailable
                                }
                              >
                                {tunnelStartMutation.isPending
                                  ? "Starting..."
                                  : getTunnelStartButtonLabel(
                                      selectedTunnelMeta,
                                      hasCustomMasterPassword,
                                    )}
                              </Button>
                              <Button
                                size="small"
                                onClick={handleTunnelStop}
                                disabled={
                                  tunnelStopMutation.isPending ||
                                  !toBool(tunnel.active)
                                }
                              >
                                {tunnelStopMutation.isPending
                                  ? "Stopping..."
                                  : getTunnelStopButtonLabel(
                                      selectedTunnelMeta,
                                    )}
                              </Button>
                              <Button
                                size="small"
                                onClick={async () => {
                                  const url = str(tunnel.url, "");
                                  if (!url) return;
                                  await navigator.clipboard.writeText(url);
                                  setSuccess("Tunnel URL copied.");
                                }}
                                disabled={!str(tunnel.url, "").trim()}
                              >
                                Copy link
                              </Button>
                              <Button
                                size="small"
                                variant="outlined"
                                onClick={() => {
                                  const url = str(tunnel.url, "").trim();
                                  if (!url) return;
                                  window.open(
                                    url,
                                    "_blank",
                                    "noopener,noreferrer",
                                  );
                                }}
                                disabled={!str(tunnel.url, "").trim()}
                              >
                                Open link
                              </Button>
                            </Stack>
                            {advancedTunnelConfigFields.length > 0 ? (
                              <Accordion
                                expanded={showTunnelAdvanced}
                                onChange={(_, expanded) =>
                                  setShowTunnelAdvanced(expanded)
                                }
                                disableGutters
                                sx={{
                                  background: "transparent",
                                  boxShadow: "none",
                                  border: "1px solid var(--ui-rgba-62-143-214-180)",
                                  borderRadius: 1,
                                }}
                              >
                                <AccordionSummary
                                  expandIcon={<ExpandMoreIcon />}
                                >
                                  <Typography
                                    variant="body2"
                                    sx={{ fontWeight: 600 }}
                                  >
                                    Advanced configuration
                                  </Typography>
                                </AccordionSummary>
                                <AccordionDetails sx={{ pt: 0 }}>
                                  <Stack spacing={1}>
                                    {advancedTunnelConfigFields.map((field: any) => {
                                      const key = str(field.key, "");
                                      const inputType = str(
                                        field.input_type,
                                        "text",
                                      );
                                      const value =
                                        tunnelDraftValues[key] ?? "";
                                      return (
                                        <TextField
                                          key={key}
                                          label={str(
                                            field.label,
                                            key || "Field",
                                          )}
                                          value={value}
                                          onChange={(e) =>
                                            setTunnelDraftValues((prev: any) => ({
                                              ...prev,
                                              [key]: e.target.value,
                                            }))
                                          }
                                          fullWidth
                                          size="small"
                                          required={toBool(field.required)}
                                          placeholder={
                                            str(field.placeholder, "") ||
                                            undefined
                                          }
                                          type={
                                            inputType === "password"
                                              ? "password"
                                              : "text"
                                          }
                                        />
                                      );
                                    })}
                                  </Stack>
                                </AccordionDetails>
                              </Accordion>
                            ) : null}
                            {hasCustomMasterPassword &&
                            getTunnelPanelWarning(selectedTunnelMeta) ? (
                              <Typography
                                variant="caption"
                                sx={{
                                  color: "text.secondary",
                                }}
                              >
                                {getTunnelPanelWarning(selectedTunnelMeta)}
                              </Typography>
                            ) : null}
                          </Stack>
                        )}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "vault" }}
                    >
                      <Stack spacing={1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Secrets vault",
                          description: vaultSummaryText,
                          info: "Save private keys and tokens here so AgentArk can use them without showing the raw value in normal screens.",
                          icon: <InventoryRoundedIcon fontSize="small" />,
                          action: (
                            <Chip
                              size="small"
                              variant="outlined"
                              label={`${vaultSecrets.length} saved`}
                            />
                          ),
                        })}
                        {hasCustomMasterPassword ? (
                          <TextField
                            label="Master password for protected edits"
                            value={vaultPassword}
                            onChange={(e) => setVaultPassword(e.target.value)}
                            fullWidth
                            size="small"
                            type="password"
                          />
                        ) : (
                          <Typography
                            variant="caption"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Using built-in local encryption. Set a custom
                            password only if you want password-protected sign-in
                            and remote access.
                          </Typography>
                        )}
                        <Stack
                          direction={{ xs: "column", sm: "row" }}
                          spacing={1}
                        >
                          <Button
                            size="small"
                            onClick={async () => {
                              setError(null);
                              await queryClient.invalidateQueries({
                                queryKey: ["settings-secrets"],
                              });
                            }}
                            disabled={vaultSecretsQ.isLoading}
                          >
                            Refresh
                          </Button>
                          <Button
                            size="small"
                            variant="outlined"
                            onClick={openVaultEditor}
                          >
                            Add Custom Secret
                          </Button>
                        </Stack>

                        {vaultSecretsQ.isLoading ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Loading secrets...
                          </Typography>
                        ) : vaultSecretsQ.error ? (
                          <Alert severity="error">
                            {errMessage(vaultSecretsQ.error)}
                          </Alert>
                        ) : vaultSecrets.length === 0 ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            No encrypted secrets stored yet.
                          </Typography>
                        ) : (
                          <TableContainer
                            className="table-shell"
                            sx={{ width: "100%", overflowX: "auto" }}
                          >
                            <Table
                              size="small"
                              sx={{ tableLayout: "fixed", width: "100%" }}
                            >
                              <TableHead>
                                <TableRow>
                                  <TableCell sx={{ width: "35%" }}>
                                    Key
                                  </TableCell>
                                  <TableCell sx={{ width: "20%" }}>
                                    Source
                                  </TableCell>
                                  <TableCell sx={{ width: "25%" }}>
                                    Value
                                  </TableCell>
                                  <TableCell
                                    sx={{ width: "20%" }}
                                    align="right"
                                  >
                                    Ops
                                  </TableCell>
                                </TableRow>
                              </TableHead>
                              <TableBody>
                                {vaultSecrets.map((row: any, idx: number) => {
                                  const key = str(row.key, "");
                                  const storageKey = str(row.storage_key, key);
                                  const displayKey = str(row.key, storageKey);
                                  const shownValue = str(row.masked, "");
                                  const source = str(row.source, "custom");
                                  const sourceLabel = str(row.source_label, "")
                                    .trim();
                                  const deletable = toBool(row.deletable);
                                  return (
                                    <TableRow key={`${storageKey}-${idx}`}>
                                      <TableCell
                                        sx={{
                                          fontFamily:
                                            "ui-monospace, SFMono-Regular, Menlo, monospace",
                                          fontSize: "0.8rem",
                                          overflow: "hidden",
                                          textOverflow: "ellipsis",
                                          whiteSpace: "nowrap",
                                        }}
                                        title={displayKey}
                                      >
                                        {displayKey}
                                      </TableCell>
                                      <TableCell sx={{ whiteSpace: "nowrap" }}>
                                        <Typography
                                          variant="body2"
                                        >
                                          {sourceLabel ||
                                            source.replace(/[-_]+/g, " ")}
                                        </Typography>
                                      </TableCell>
                                      <TableCell sx={{ overflow: "hidden" }}>
                                        <Typography
                                          variant="body2"
                                          title={shownValue}
                                          sx={{
                                            whiteSpace: "nowrap",
                                            overflow: "hidden",
                                            textOverflow: "ellipsis",
                                          }}
                                        >
                                          {shownValue || "-"}
                                        </Typography>
                                      </TableCell>
                                      <TableCell
                                        align="right"
                                        sx={{ whiteSpace: "nowrap" }}
                                      >
                                        <Stack
                                          direction="row"
                                          spacing={0.5}
                                          sx={{
                                            justifyContent: "flex-end",
                                          }}
                                        >
                                          {deletable ? (
                                            <Button
                                              size="small"
                                              color="error"
                                              sx={{
                                                minWidth: 72,
                                                whiteSpace: "nowrap",
                                              }}
                                              onClick={async () => {
                                                const ok = window.confirm(
                                                  `Delete secret '${displayKey}'?`,
                                                );
                                                if (!ok) return;
                                                const pw =
                                                  resolveVaultPasswordForSensitiveOps();
                                                if (pw === null) return;
                                                setError(null);
                                                try {
                                                  await deleteVaultSecretMutation.mutateAsync(
                                                    {
                                                      key: storageKey,
                                                      password: pw || undefined,
                                                    },
                                                  );
                                                } catch {
                                                  // handled by mutation onError
                                                }
                                              }}
                                              disabled={
                                                deleteVaultSecretMutation.isPending
                                              }
                                            >
                                              Delete
                                            </Button>
                                          ) : (
                                            <Typography
                                              variant="caption"
                                              sx={{
                                                color: "text.secondary",
                                              }}
                                            >
                                              Managed elsewhere
                                            </Typography>
                                          )}
                                        </Stack>
                                      </TableCell>
                                    </TableRow>
                                  );
                                })}
                              </TableBody>
                            </Table>
                          </TableContainer>
                        )}
                      </Stack>
                    </Box>

                    <Box
                      className="list-shell"
                      sx={{ minHeight: 0, gridArea: "privacy" }}
                    >
                      <Stack spacing={1.5}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Security",
                          title: "Model Privacy Boundary",
                          description:
                            "Control what sensitive content can enter model prompts, and whether chat can pause for one-time approval when read-only tools uncover person-linked data.",
                          icon: <VisibilityOffRoundedIcon fontSize="small" />,
                          action: (
                            <Chip
                              size="small"
                              color={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                                  ? "warning"
                                  : "default"
                              }
                              variant="outlined"
                              label={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                                  ? "Approval cards on"
                                  : "Approval cards off"
                              }
                            />
                          ),
                        })}
                        <Alert severity="info" sx={{ py: 0.25 }}>
                          Secrets are still never sent to the model. When
                          approval cards are enabled, chat pauses and shows
                          Approve/Reject buttons before the model can inspect
                          sensitive read-only tool results for a single request.
                        </Alert>
                        <Grid2 container spacing={1.5}>
                          <Grid2 size={{ xs: 12, md: 6 }}>
                            <TextField
                              fullWidth
                              select
                              size="small"
                              label="Retrieved context handling"
                              value={form.model_privacy_default_mode}
                              onChange={(e) =>
                                setField(
                                  "model_privacy_default_mode",
                                  e.target.value,
                                )
                              }
                              helperText="Applies to history, memories, tool output, documents, and helper-model prompts."
                            >
                              <MenuItem value="default_redact">
                                Default redact
                              </MenuItem>
                              <MenuItem value="zero_exposure">
                                Zero exposure
                              </MenuItem>
                              <MenuItem value="secrets_only">
                                Secrets only
                              </MenuItem>
                            </TextField>
                          </Grid2>
                          <Grid2 size={{ xs: 12, md: 6 }}>
                            <TextField
                              fullWidth
                              select
                              size="small"
                              label="Current chat handling"
                              value={form.model_privacy_current_chat_pii_policy}
                              onChange={(e) =>
                                setField(
                                  "model_privacy_current_chat_pii_policy",
                                  e.target.value,
                                )
                              }
                              helperText="Choose whether the active user message stays raw, gets masked, or is blocked when sensitive."
                            >
                              <MenuItem value="raw_current_turn">
                                Raw current turn
                              </MenuItem>
                              <MenuItem value="mask_chat_pii">
                                Mask chat PII
                              </MenuItem>
                              <MenuItem value="block_sensitive_chat">
                                Block sensitive chat
                              </MenuItem>
                            </TextField>
                          </Grid2>
                        </Grid2>
                        <FormControlLabel
                          control={
                            <Switch
                              checked={
                                form.model_privacy_request_scoped_sensitive_approval_enabled
                              }
                              onChange={(e) =>
                                setField(
                                  "model_privacy_request_scoped_sensitive_approval_enabled",
                                  e.target.checked,
                                )
                              }
                            />
                          }
                          label="Show approve/reject cards for sensitive read-only tool results"
                        />
                        <Typography
                          variant="caption"
                          sx={{
                            color: "text.secondary",
                          }}
                        >
                          Approvals are request-scoped only. Reject keeps the
                          data masked. Approve reveals non-secret sensitive
                          context for that single follow-up turn.
                        </Typography>
                      </Stack>
                    </Box>
                  </Box>
              </Stack>
  );
}
