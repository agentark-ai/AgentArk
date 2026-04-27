import InfoOutlinedIcon from "@mui/icons-material/InfoOutlined";
import { Box, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import { Suspense, type ReactNode } from "react";

export type SettingsSectionIntroArgs = {
  eyebrow: string;
  title: string;
  description: string;
  info?: string | null;
  action?: ReactNode;
};

export type SettingsInlineCardProps = {
  eyebrow?: string;
  title: string;
  description: string;
  tone?: "default" | "info" | "warning";
  fullWidthCopy?: boolean;
  action?: ReactNode;
  children?: ReactNode;
};

export function normalizeSettingsHeading(value: string): string {
  return value
    .trim()
    .toLowerCase()
    .replace(/&/g, "and")
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

export function WorkspaceLazyPanel({
  children,
  message = "Loading panel...",
}: {
  children: ReactNode;
  message?: string;
}) {
  return (
    <Suspense
      fallback={
        <Box className="list-shell" sx={{ minHeight: 180, p: 1.5 }}>
          <Typography variant="body2" sx={{ color: "text.secondary" }}>
            {message}
          </Typography>
        </Box>
      }
    >
      {children}
    </Suspense>
  );
}

export function SettingsSectionIntro({
  eyebrow,
  title,
  description,
  info = null,
  action = null,
  selectedHeaderTitle,
}: SettingsSectionIntroArgs & { selectedHeaderTitle: string }) {
  const showEyebrow =
    normalizeSettingsHeading(eyebrow) !==
    normalizeSettingsHeading(selectedHeaderTitle);
  const duplicatesPageHeader =
    !info &&
    !action &&
    normalizeSettingsHeading(title) ===
      normalizeSettingsHeading(selectedHeaderTitle);

  if (duplicatesPageHeader) {
    return null;
  }

  return (
    <Stack
      className="settings-section-intro"
      direction={{ xs: "column", md: "row" }}
      spacing={1}
      sx={{
        justifyContent: "space-between",
        alignItems: { xs: "flex-start", md: "center" },
      }}
    >
      <Box className="settings-section-intro-copy">
        {showEyebrow ? (
          <Typography className="settings-section-kicker">
            {eyebrow}
          </Typography>
        ) : null}
        <Stack
          direction="row"
          spacing={0.75}
          className="settings-section-title-row"
          sx={{
            alignItems: "center",
          }}
        >
          <Typography className="settings-section-title">{title}</Typography>
          {info ? (
            <Tooltip title={info} arrow placement="top-start">
              <IconButton
                size="small"
                className="settings-section-info"
                aria-label={`${title} information`}
              >
                <InfoOutlinedIcon fontSize="inherit" />
              </IconButton>
            </Tooltip>
          ) : null}
        </Stack>
        <Typography className="settings-section-description">
          {description}
        </Typography>
      </Box>
      {action ? <Box className="settings-section-actions">{action}</Box> : null}
    </Stack>
  );
}

export function SettingsInlineCard({
  eyebrow,
  title,
  description,
  tone = "default",
  fullWidthCopy = false,
  action = null,
  children = null,
}: SettingsInlineCardProps) {
  return (
    <Box className={`settings-inline-card tone-${tone}`}>
      <Stack
        className="settings-inline-card-head"
        direction={{ xs: "column", md: "row" }}
        spacing={1}
        sx={{
          justifyContent: "space-between",
          alignItems: { xs: "flex-start", md: "center" },
        }}
      >
        <Box
          className="settings-inline-card-copy"
          sx={fullWidthCopy ? { maxWidth: "none", flex: 1 } : undefined}
        >
          {eyebrow ? (
            <Typography className="settings-inline-card-kicker">
              {eyebrow}
            </Typography>
          ) : null}
          <Typography className="settings-inline-card-title">
            {title}
          </Typography>
          <Typography
            className="settings-inline-card-description"
            sx={fullWidthCopy ? { maxWidth: "none" } : undefined}
          >
            {description}
          </Typography>
        </Box>
        {action ? (
          <Box className="settings-inline-card-actions">{action}</Box>
        ) : null}
      </Stack>
      {children ? (
        <Box className="settings-inline-card-body">{children}</Box>
      ) : null}
    </Box>
  );
}
