import { Box, Stack, Typography, type SxProps, type Theme } from "@mui/material";
import type { ReactNode } from "react";

type WorkspacePageShellProps = {
  children: ReactNode;
  spacing?: number;
  className?: string;
  sx?: SxProps<Theme>;
};

type WorkspacePageHeaderProps = {
  eyebrow: string;
  title: string;
  description: ReactNode;
  actions?: ReactNode;
  className?: string;
  sx?: SxProps<Theme>;
};

export function WorkspacePageShell({
  children,
  spacing = 1.5,
  className = "",
  sx,
}: WorkspacePageShellProps) {
  return (
    <Stack
      spacing={spacing}
      className={["workspace-page-shell", className].filter(Boolean).join(" ")}
      sx={{ minWidth: 0, width: "100%", ...sx }}
    >
      {children}
    </Stack>
  );
}

export function WorkspacePageHeader({
  eyebrow,
  title,
  description,
  actions = null,
  className = "",
  sx,
}: WorkspacePageHeaderProps) {
  return (
    <Box
      className={["list-shell", "workspace-page-hero-shell", className].filter(Boolean).join(" ")}
      sx={sx}
    >
      <Stack
        direction={{ xs: "column", md: "row" }}
        spacing={1}
        className="workspace-page-header"
        sx={{
          justifyContent: "space-between",
          alignItems: { xs: "stretch", md: "flex-start" }
        }}>
        <Box className="workspace-page-header-copy">
          <Typography className="workspace-page-kicker">{eyebrow}</Typography>
          <Typography className="workspace-page-title">{title}</Typography>
          <Typography component="div" className="workspace-page-copy">
            {description}
          </Typography>
        </Box>
        {actions ? <Box className="workspace-page-header-actions">{actions}</Box> : null}
      </Stack>
    </Box>
  );
}
