import ExpandMoreIcon from "@mui/icons-material/ExpandMore";
import LockRoundedIcon from "@mui/icons-material/LockRounded";
import ManageAccountsRoundedIcon from "@mui/icons-material/ManageAccountsRounded";
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
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
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
import { useState } from "react";
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
import { SenderVerificationPanel } from "../SenderVerificationPanel";

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
  vaultSecretsRequested,
  requestVaultSecrets,
  queryClient,
  openVaultEditor,
  deleteVaultSecretMutation,
  resolveVaultPasswordForSensitiveOps,
  autoRefresh,
  form,
  setField,
  setError,
  setSuccess,
}: SettingsSecurityPanelProps) {
  const [remoteSetupOpen, setRemoteSetupOpen] = useState(false);
  const [internalCredentialsOpen, setInternalCredentialsOpen] = useState(false);
  const [vaultOpen, setVaultOpen] = useState(false);
  const [privacyOpen, setPrivacyOpen] = useState(false);
  const [senderVerificationOpen, setSenderVerificationOpen] = useState(false);
  const securityStatusLoading = securityStatusQ.isLoading;
  const remoteAccessLoading = tunnelQ.isLoading || tunnelProvidersQ.isLoading;
  const remoteAccessActive = toBool(tunnel.active);
  const passwordStatusLabel = securityStatusLoading
    ? "Checking..."
    : hasCustomMasterPassword
      ? "Password required"
      : "No sign-in password";
  const passwordStatusTone = securityStatusLoading
    ? "default"
    : hasCustomMasterPassword
      ? "success"
      : "warning";
  const passwordHelpText = securityStatusLoading
    ? "Checking whether this instance already requires a password."
    : hasCustomMasterPassword
    ? "People must enter your AgentArk password before they can use this instance."
    : "Local data is encrypted, but anyone on this computer can open AgentArk. Add a password before using remote access.";
  const remoteAccessHelpText = remoteAccessLoading
    ? "Checking the remote access provider before showing the current state."
    : remoteAccessActive
    ? `${tunnelStateLabel}. Stop it when you are done using another device.`
    : hasCustomMasterPassword
      ? "Off by default. Turn it on only when you need AgentArk from another device."
      : "Off. Add a sign-in password before opening AgentArk from another device.";
  const remoteAccessPrimaryLabel = remoteAccessActive
    ? "Manage access"
    : tunnelStartMutation.isPending
      ? "Turning on..."
      : hasCustomMasterPassword
        ? "Configure and turn on"
        : "Add password and turn on";
  const reviewStatusLabel =
    abuseReviews.length > 0 ? `${abuseReviews.length} to review` : "Nothing waiting";
  const remoteSetupNeedsAttention = Boolean(
    tunnelQ.error ||
      tunnelProvidersQ.error ||
      tunnelPanelNotice ||
      str(tunnel.error, "").trim() ||
      tunnelSetupChecks.length > 0 ||
      (!remoteAccessLoading && hasCustomMasterPassword && !selectedTunnelAvailable),
  );
  const remoteStatusLabel = remoteAccessLoading
    ? "Checking..."
    : remoteAccessActive
      ? "On"
      : remoteSetupNeedsAttention
        ? "Needs setup"
      : "Off";
  const vaultStatusLabel =
    !vaultSecretsRequested
      ? "Not loaded"
      : vaultSecretsQ.isLoading
        ? "Checking..."
        : vaultSecrets.length === 0
          ? "No saved secrets"
          : `${vaultSecrets.length} saved`;

  return (
    <>
              <Stack spacing={2} className="settings-security-panel">
                <Box className="security-basics-panel">
                  <Stack spacing={1.25}>
                    <Box className="security-basics-heading">
                      <Typography className="security-basics-kicker">
                        Start here
                      </Typography>
                      <Typography className="security-basics-title">
                        Security basics
                      </Typography>
                      <Typography className="security-basics-description">
                        Most people only need these three checks. Advanced setup is below.
                      </Typography>
                    </Box>
                    <Box className="security-basics-list">
                      <Box className="security-basics-row">
                        <Box className="security-basics-icon">
                          <LockRoundedIcon fontSize="small" />
                        </Box>
                        <Box className="security-basics-copy">
                          <Typography className="security-basics-row-title">
                            Protect sign-in
                          </Typography>
                          <Typography className="security-basics-row-copy">
                            {passwordHelpText}
                          </Typography>
                        </Box>
                        <Chip
                          size="small"
                          color={passwordStatusTone}
                          variant="outlined"
                          label={passwordStatusLabel}
                        />
                        {securityStatusLoading ? null : hasCustomMasterPassword ? (
                          <Stack direction="row" spacing={0.75} useFlexGap>
                            <Button
                              size="small"
                              variant="outlined"
                              onClick={() => openPasswordDialog("change")}
                              disabled={passwordMutationPending}
                            >
                              Change
                            </Button>
                            <Button
                              size="small"
                              color="error"
                              variant="text"
                              onClick={() => openPasswordDialog("remove")}
                              disabled={passwordMutationPending}
                            >
                              Remove
                            </Button>
                          </Stack>
                        ) : (
                          <Button
                            size="small"
                            variant="contained"
                            onClick={() => openPasswordDialog("set")}
                            disabled={passwordMutationPending}
                          >
                            Add password
                          </Button>
                        )}
                      </Box>
                      <Box className="security-basics-row">
                        <Box className="security-basics-icon">
                          <PublicRoundedIcon fontSize="small" />
                        </Box>
                        <Box className="security-basics-copy">
                          <Typography className="security-basics-row-title">
                            Remote access
                          </Typography>
                          <Typography className="security-basics-row-copy">
                            {remoteAccessHelpText}
                          </Typography>
                        </Box>
                        <Chip
                          size="small"
                          color={remoteAccessLoading ? "default" : tunnelSummaryTone}
                          variant="outlined"
                          label={remoteStatusLabel}
                        />
                        <Button
                          size="small"
                          variant={
                            remoteAccessActive || !hasCustomMasterPassword
                              ? "outlined"
                              : "contained"
                          }
                          onClick={() => setRemoteSetupOpen(true)}
                          disabled={remoteAccessLoading}
                        >
                          {remoteAccessPrimaryLabel}
                        </Button>
                      </Box>
                      <Box className="security-basics-row">
                        <Box className="security-basics-icon">
                          <ShieldRoundedIcon fontSize="small" />
                        </Box>
                        <Box className="security-basics-copy">
                          <Typography className="security-basics-row-title">
                            Review blocked senders
                          </Typography>
                          <Typography className="security-basics-row-copy">
                            AgentArk pauses suspicious inbound senders here instead of guessing.
                          </Typography>
                        </Box>
                        <Chip
                          size="small"
                          color={abuseReviews.length > 0 ? "warning" : "success"}
                          variant="outlined"
                          label={reviewStatusLabel}
                        />
                      </Box>
                      <Box className="security-basics-row">
                        <Box className="security-basics-icon">
                          <ManageAccountsRoundedIcon fontSize="small" />
                        </Box>
                        <Box className="security-basics-copy">
                          <Typography className="security-basics-row-title">
                            Sender verification
                          </Typography>
                          <Typography className="security-basics-row-copy">
                            Trust policies, pending approvals, and approved senders for inbound channels.
                          </Typography>
                        </Box>
                        <Chip
                          size="small"
                          color="info"
                          variant="outlined"
                          label="Inbound trust"
                        />
                        <Button
                          size="small"
                          variant="outlined"
                          onClick={() => setSenderVerificationOpen(true)}
                        >
                          Manage
                        </Button>
                      </Box>
                    </Box>
                  </Stack>
                </Box>
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
                      sx={{ minHeight: 0 }}
                    >
                      <Stack spacing={1.1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Messages",
                          title: "Blocked sender details",
                          description:
                            "If AgentArk pauses a sender, review the source and choose whether to keep it paused or allow it again.",
                          icon: <ShieldRoundedIcon fontSize="small" />,
                          action: (
                            <Chip
                              size="small"
                              color={abuseReviews.length > 0 ? "warning" : "success"}
                              variant="outlined"
                              label={
                                abuseReviews.length > 0
                                  ? `${abuseReviews.length} waiting for you`
                                  : "All clear"
                              }
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
                          <Typography
                            variant="body2"
                            sx={{ color: "text.secondary", py: 0.5 }}
                          >
                            Nothing to review right now.
                          </Typography>
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
                      <Accordion
                        className="security-accordion"
                        expanded={internalCredentialsOpen}
                        onChange={(_, expanded) =>
                          setInternalCredentialsOpen(expanded)
                        }
                        disableGutters
                      >
                        <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                          <Box className="security-accordion-summary">
                            <Typography className="security-accordion-title">
                              Internal service credentials
                            </Typography>
                            <Typography className="security-accordion-copy">
                              Expert-only keys used between AgentArk services.
                            </Typography>
                          </Box>
                        </AccordionSummary>
                        <AccordionDetails>
                          <Box
                            className="list-shell security-panel-details"
                            sx={{ minHeight: 0 }}
                          >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Service",
                          title: "Service credentials",
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
                        </AccordionDetails>
                      </Accordion>
                    ) : null}

                    <Dialog
                      open={remoteSetupOpen}
                      onClose={() => setRemoteSetupOpen(false)}
                      fullWidth
                      maxWidth="md"
                    >
                      <DialogTitle>Remote access setup</DialogTitle>
                      <DialogContent dividers sx={{ p: 1.5 }}>
                        <Box
                          className="list-shell security-panel-details"
                          sx={{ minHeight: 0 }}
                        >
                      <Stack spacing={1.25}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Access",
                          title: "Remote access",
                          description:
                            "Reach AgentArk from another device. We'll only open it up while you say so.",
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
                            <TextField
                              label="Access method"
                              select
                              size="small"
                              fullWidth
                              helperText="Provider auth fields appear below when needed."
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
                            <Stack spacing={0.25} sx={{ px: 0.25 }}>
                              <Typography
                                variant="body2"
                                sx={{ fontWeight: 600 }}
                              >
                                {tunnelPrimaryText}
                              </Typography>
                              {tunnelPrimaryDetail ? (
                                <Typography
                                  variant="caption"
                                  sx={{ color: "text.secondary" }}
                                >
                                  {tunnelPrimaryDetail}
                                </Typography>
                              ) : null}
                            </Stack>
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
                            {/*
                              Buttons grouped into two semantic rows:
                                Row 1 — Configure: Save + Check setup
                                Row 2 — Lifecycle: Start (primary) / Stop, plus
                                        Copy / Open helpers only shown when a
                                        URL actually exists.
                            */}
                            <Stack spacing={1}>
                              <Stack
                                direction={{ xs: "column", sm: "row" }}
                                spacing={1}
                                useFlexGap
                                sx={{ flexWrap: "wrap" }}
                              >
                                <Button
                                  size="small"
                                  variant="outlined"
                                  onClick={handleTunnelProviderSave}
                                  disabled={tunnelSaveMutation.isPending}
                                >
                                  {tunnelSaveMutation.isPending
                                    ? "Saving..."
                                    : "Save configuration"}
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
                              </Stack>
                              <Stack
                                direction={{ xs: "column", sm: "row" }}
                                spacing={1}
                                useFlexGap
                                sx={{ flexWrap: "wrap", alignItems: "center" }}
                              >
                                {toBool(tunnel.active) ? (
                                  <Button
                                    size="small"
                                    color="error"
                                    variant="outlined"
                                    onClick={handleTunnelStop}
                                    disabled={tunnelStopMutation.isPending}
                                  >
                                    {tunnelStopMutation.isPending
                                      ? "Stopping..."
                                      : getTunnelStopButtonLabel(
                                          selectedTunnelMeta,
                                        )}
                                  </Button>
                                ) : (
                                  <Button
                                    size="small"
                                    variant="contained"
                                    onClick={handleTunnelStart}
                                    disabled={
                                      tunnelSaveMutation.isPending ||
                                      tunnelStartMutation.isPending ||
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
                                )}
                                {str(tunnel.url, "").trim() ? (
                                  <>
                                    <Box
                                      sx={{
                                        width: 1,
                                        height: 22,
                                        background:
                                          "var(--ui-rgba-255-255-255-080)",
                                        mx: 0.25,
                                      }}
                                    />
                                    <Button
                                      size="small"
                                      onClick={async () => {
                                        const url = str(tunnel.url, "");
                                        if (!url) return;
                                        await navigator.clipboard.writeText(url);
                                        setSuccess("Tunnel URL copied.");
                                      }}
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
                                    >
                                      Open link
                                    </Button>
                                  </>
                                ) : null}
                              </Stack>
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
                      </DialogContent>
                      <DialogActions>
                        <Button onClick={() => setRemoteSetupOpen(false)}>
                          Done
                        </Button>
                      </DialogActions>
                    </Dialog>

                    <Accordion
                      className="security-accordion"
                      expanded={vaultOpen}
                      onChange={(_, expanded) => {
                        setVaultOpen(expanded);
                        if (expanded) requestVaultSecrets();
                      }}
                      disableGutters
                    >
                      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                        <Box className="security-accordion-summary">
                          <Typography className="security-accordion-title">
                            Stored secrets
                          </Typography>
                          <Typography className="security-accordion-copy">
                            {vaultStatusLabel}. API keys and tokens stay encrypted.
                          </Typography>
                        </Box>
                      </AccordionSummary>
                      <AccordionDetails>
                        <Box
                          className="list-shell security-panel-details"
                          sx={{ minHeight: 0 }}
                        >
                      <Stack spacing={1}>
                        {renderSettingsSectionIntro({
                          eyebrow: "Vault",
                          title: "Stored secrets",
                          description:
                            "API keys, tokens, and credentials AgentArk uses for you. Encrypted at rest, never shown in plain text.",
                          icon: <InventoryRoundedIcon fontSize="small" />,
                          action: (
                            <Chip
                              size="small"
                              variant="outlined"
                              label={vaultStatusLabel}
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

                        {!vaultSecretsRequested ? (
                          <Typography
                            variant="body2"
                            sx={{
                              color: "text.secondary",
                            }}
                          >
                            Opened on demand to keep Security settings fast.
                          </Typography>
                        ) : vaultSecretsQ.isLoading ? (
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
                      </AccordionDetails>
                    </Accordion>

                    <Accordion
                      className="security-accordion"
                      expanded={privacyOpen}
                      onChange={(_, expanded) => setPrivacyOpen(expanded)}
                      disableGutters
                    >
                      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
                        <Box className="security-accordion-summary">
                          <Typography className="security-accordion-title">
                            Advanced model privacy
                          </Typography>
                          <Typography className="security-accordion-copy">
                            Controls for what sensitive context can be sent to models.
                          </Typography>
                        </Box>
                      </AccordionSummary>
                      <AccordionDetails>
                        <Box
                          className="list-shell security-panel-details"
                          sx={{ minHeight: 0 }}
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
                      </AccordionDetails>
                    </Accordion>
                  </Box>
              </Stack>
      <Dialog
        open={senderVerificationOpen}
        onClose={() => setSenderVerificationOpen(false)}
        maxWidth="lg"
        fullWidth
        slotProps={{
          paper: {
            className: "sender-verification-dialog",
            sx: {
              maxHeight: "min(88vh, 920px)",
            },
          },
        }}
      >
        <DialogTitle>
          <Stack
            direction={{ xs: "column", sm: "row" }}
            spacing={1}
            sx={{
              alignItems: { xs: "flex-start", sm: "center" },
              justifyContent: "space-between",
            }}
          >
            <Box>
              <Typography variant="h6" sx={{ lineHeight: 1.2 }}>
                Sender Verification
              </Typography>
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                Edit inbound sender policies and approvals.
              </Typography>
            </Box>
            <Button
              size="small"
              variant="outlined"
              onClick={() => setSenderVerificationOpen(false)}
            >
              Close
            </Button>
          </Stack>
        </DialogTitle>
        <DialogContent dividers>
          <SenderVerificationPanel autoRefresh={Boolean(autoRefresh)} />
        </DialogContent>
      </Dialog>
    </>
  );
}
