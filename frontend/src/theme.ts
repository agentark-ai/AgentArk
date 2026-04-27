import { createTheme } from "@mui/material";

export const appTheme = createTheme({
  spacing: 8,
  palette: {
    mode: "dark",
    primary: {
      main: "#d8ad78",
      light: "#f1d6ad",
      dark: "#8d6841",
    },
    secondary: {
      main: "#f0e2cf",
      light: "#fff4e5",
    },
    success: {
      main: "#8bd6a5",
    },
    warning: {
      main: "#d8ad78",
    },
    error: {
      main: "#ff9b9b",
    },
    info: {
      main: "#d8c2a6",
    },
    background: {
      default: "#0a0a0a",
      paper: "#111111",
    },
    text: {
      primary: "#f6f0e8",
      secondary: "rgba(226, 218, 208, 0.72)",
    },
    divider: "rgba(255, 255, 255, 0.10)",
  },
  shape: {
    borderRadius: 8,
  },
  typography: {
    fontFamily: "'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
    fontSize: 14,
    h3: {
      fontFamily: "'JetBrains Mono', 'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 600,
      fontSize: "1.6rem",
      lineHeight: 1.24,
      letterSpacing: 0,
    },
    h4: {
      fontFamily: "'JetBrains Mono', 'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 600,
      fontSize: "1.4rem",
      lineHeight: 1.28,
      letterSpacing: 0,
    },
    h5: {
      fontFamily: "'JetBrains Mono', 'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 600,
      fontSize: "1.2rem",
      lineHeight: 1.32,
      letterSpacing: 0,
    },
    h6: {
      fontFamily: "'JetBrains Mono', 'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 600,
      fontSize: "1rem",
      lineHeight: 1.34,
    },
    subtitle1: {
      fontFamily: "'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.9rem",
      fontWeight: 600,
      lineHeight: 1.4,
    },
    subtitle2: {
      fontFamily: "'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.82rem",
      fontWeight: 600,
      lineHeight: 1.4,
    },
    body1: {
      fontFamily: "'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.94rem",
      lineHeight: 1.58,
    },
    body2: {
      fontFamily: "'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.88rem",
      lineHeight: 1.54,
    },
    caption: {
      fontFamily: "'JetBrains Mono', 'Inter', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.72rem",
      lineHeight: 1.4,
      color: "var(--ui-rgba-213-216-223-680)",
    },
  },
  components: {
    MuiCssBaseline: {
      styleOverrides: {
        body: {
          scrollbarWidth: "thin",
          scrollbarColor: "var(--ui-rgba-0-255-170-150) transparent",
        },
      },
    },
    MuiCard: {
      styleOverrides: {
        root: {
          border: "1px solid var(--surface-border)",
          background: "var(--surface-bg-elevated)",
          backdropFilter: "blur(8px)",
          borderRadius: 8,
          transition: "border-color 0.25s ease, box-shadow 0.25s ease, transform 0.2s ease",
          "&:hover": {
            borderColor: "var(--surface-border-strong)",
            boxShadow: "var(--surface-shadow-soft)",
          },
        },
      },
    },
    MuiCardContent: {
      styleOverrides: {
        root: {
          padding: "12px",
          "&:last-child": {
            paddingBottom: "12px",
          },
        },
      },
    },
    MuiButton: {
      defaultProps: {
        size: "small",
        disableElevation: true,
      },
      styleOverrides: {
        root: {
          textTransform: "none" as const,
          fontWeight: 600,
          fontSize: "0.78rem",
          lineHeight: 1.16,
          letterSpacing: 0,
          fontFamily: "var(--font-mono)",
          borderRadius: "var(--button-radius)",
          minHeight: "var(--button-height-sm)",
          padding: "0 var(--button-pad-x-sm)",
          border: "1px solid transparent",
          transition: "background 0.18s ease, border-color 0.18s ease, color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease",
          "&:active": {
            transform: "translateY(0) scale(0.98)",
            background: "var(--button-bg-pressed)",
          },
          "&.Mui-disabled": {
            color: "var(--ui-rgba-170-193-220-380)",
            borderColor: "var(--ui-rgba-95-132-172-140)",
            background: "var(--ui-rgba-10-18-31-420)",
            boxShadow: "none",
          },
          variants: [
            {
              props: { variant: "contained", color: "secondary" },
              style: {
                background: "var(--button-bg-subtle)",
                color: "var(--button-text)",
                borderColor: "var(--button-border)",
                boxShadow: "var(--button-shadow)",
                "&:hover": {
                  background: "var(--button-bg-subtle-hover)",
                  borderColor: "var(--button-border-strong)",
                  boxShadow: "var(--button-shadow-hover)",
                },
              },
            },
            {
              props: { variant: "contained", color: "success" },
              style: { color: "#07131f" },
            },
            {
              props: { variant: "contained", color: "warning" },
              style: { color: "#07131f" },
            },
            {
              props: { variant: "contained", color: "error" },
              style: { color: "#f7fbff" },
            },
            {
              props: { variant: "outlined", color: "primary" },
              style: { color: "var(--button-text)" },
            },
            {
              props: { variant: "outlined", color: "success" },
              style: {
                borderColor: "var(--ui-rgba-74-210-157-280)",
                color: "#79f0bb",
                "&:hover": {
                  borderColor: "var(--ui-rgba-74-210-157-420)",
                  background: "var(--ui-rgba-9-37-29-820)",
                },
              },
            },
            {
              props: { variant: "outlined", color: "warning" },
              style: {
                borderColor: "var(--ui-rgba-255-159-67-300)",
                color: "#ffbc7c",
                "&:hover": {
                  borderColor: "var(--ui-rgba-255-159-67-440)",
                  background: "var(--ui-rgba-47-24-8-820)",
                },
              },
            },
            {
              props: { variant: "outlined", color: "error" },
              style: {
                borderColor: "var(--ui-rgba-255-107-107-300)",
                color: "#ff9f9f",
                "&:hover": {
                  borderColor: "var(--ui-rgba-255-107-107-440)",
                  background: "var(--ui-rgba-46-11-18-820)",
                },
              },
            },
            {
              props: { variant: "text", color: "primary" },
              style: { color: "var(--button-text)" },
            },
            {
              props: { variant: "text", color: "error" },
              style: { color: "#ffb0b0" },
            },
            {
              props: { variant: "text", color: "warning" },
              style: { color: "#ffc98e" },
            },
          ],
        },
        sizeSmall: {
          minHeight: "var(--button-height-sm)",
          padding: "0 var(--button-pad-x-sm)",
          fontSize: "0.78rem",
        },
        sizeMedium: {
          minHeight: "var(--button-height-md)",
          padding: "0 var(--button-pad-x-md)",
          fontSize: "0.82rem",
        },
        sizeLarge: {
          minHeight: "var(--button-height-lg)",
          padding: "0 var(--button-pad-x-lg)",
          fontSize: "0.88rem",
        },
        contained: {
          background: "var(--button-bg-primary)",
          color: "var(--button-text-strong)",
          borderColor: "var(--button-border-strong)",
          boxShadow: "var(--button-shadow-primary)",
          "&:hover": {
            background: "var(--button-bg-primary-hover)",
            borderColor: "var(--surface-border-strong)",
            boxShadow: "var(--button-shadow-hover)",
          },
        },
        outlined: {
          background: "var(--ui-rgba-22-22-26-780)",
          borderColor: "var(--button-border)",
          boxShadow: "none",
          "&:hover": {
            borderColor: "var(--button-border-strong)",
            background: "var(--button-bg-subtle-hover)",
            boxShadow: "none",
          },
        },
        text: {
          color: "var(--button-text-muted)",
          borderColor: "transparent",
          background: "transparent",
          "&:hover": {
            color: "var(--button-text)",
            background: "var(--ui-rgba-255-255-255-050)",
          },
        },
      },
    },
    MuiChip: {
      styleOverrides: {
        root: {
          fontFamily: "var(--font-mono)",
          fontSize: "0.68rem",
          fontWeight: 500,
          letterSpacing: 0,
          borderRadius: 6,
          transition: "all 0.2s ease",
          "&:hover": {
            boxShadow: "none",
          },
        },
        outlined: {
          borderColor: "var(--surface-border)",
        },
        colorSuccess: {
          "&:hover": {
            boxShadow: "none",
          },
        },
        colorError: {
          "&:hover": {
            boxShadow: "none",
          },
        },
        colorWarning: {
          "&:hover": {
            boxShadow: "none",
          },
        },
      },
    },
    MuiTableContainer: {
      styleOverrides: {
        root: {
          borderRadius: 8,
          border: "1px solid var(--surface-border)",
          background: "var(--cyber-panel)",
        },
      },
    },
    MuiTableHead: {
      styleOverrides: {
        root: {
          "& .MuiTableCell-head": {
            background: "var(--cyber-panel-raised)",
            color: "var(--ui-rgba-0-255-170-400)",
            fontFamily: "var(--font-mono)",
            fontWeight: 500,
            fontSize: "0.68rem",
            letterSpacing: 0,
            textTransform: "uppercase" as const,
            borderBottom: "1px solid var(--surface-border)",
            padding: "8px 12px",
          },
        },
      },
    },
    MuiTableBody: {
      styleOverrides: {
        root: {
          "& .MuiTableRow-root": {
            transition: "background 0.15s ease",
            "&:hover": {
              background: "var(--ui-rgba-0-255-170-040) !important",
            },
          },
          "& .MuiTableCell-body": {
            borderBottom: "1px solid var(--surface-border)",
            padding: "8px 12px",
            fontSize: "0.81rem",
            color: "var(--text-primary)",
          },
        },
      },
    },
    MuiTextField: {
      defaultProps: {
        size: "small",
      },
    },
    MuiInputBase: {
      styleOverrides: {
        root: {
          color: "var(--text-primary) !important",
        },
        input: {
          color: "var(--text-primary) !important",
          "&::placeholder": {
            color: "var(--text-dim) !important",
            opacity: "1 !important",
          },
        },
      },
    },
    MuiInputLabel: {
      styleOverrides: {
        root: {
          color: "var(--text-secondary)",
          "&.Mui-focused": {
            color: "var(--teal)",
          },
        },
      },
    },
    MuiSelect: {
      styleOverrides: {
        select: {
          color: "var(--text-primary)",
        },
        icon: {
          color: "var(--text-secondary)",
        },
      },
    },
    MuiFormHelperText: {
      styleOverrides: {
        root: {
          color: "var(--ui-rgba-214-228-255-450)",
        },
      },
    },
    MuiMenuItem: {
      styleOverrides: {
        root: {
          fontSize: "0.76rem",
          color: "var(--text-primary)",
          "&:hover": {
            background: "var(--ui-rgba-0-255-170-060)",
          },
          "&.Mui-selected": {
            background: "var(--ui-rgba-0-255-170-100)",
            "&:hover": {
              background: "var(--ui-rgba-0-255-170-150)",
            },
          },
        },
      },
    },
    MuiOutlinedInput: {
      styleOverrides: {
        root: {
          borderRadius: 8,
          fontSize: "0.82rem",
          color: "var(--text-primary)",
          "& .MuiOutlinedInput-notchedOutline": {
            borderColor: "var(--surface-border)",
            transition: "border-color 0.2s ease, box-shadow 0.2s ease",
          },
          "&:hover .MuiOutlinedInput-notchedOutline": {
            borderColor: "var(--surface-border-strong)",
          },
          "&.Mui-focused .MuiOutlinedInput-notchedOutline": {
            borderColor: "var(--teal)",
            boxShadow: "0 0 0 2px var(--ui-rgba-0-255-170-100)",
          },
        },
        input: {
          padding: "9px 11px",
          color: "var(--text-primary)",
          "&::placeholder": {
            color: "var(--text-dim)",
            opacity: 1,
          },
        },
      },
    },
    MuiDialog: {
      styleOverrides: {
        paper: {
          background: "var(--surface-bg-elevated)",
          backgroundImage: "none",
          border: "1px solid var(--surface-border)",
          borderRadius: 8,
          backdropFilter: "blur(8px)",
          boxShadow: "0 28px 96px var(--ui-rgba-0-0-0-500)",
        },
      },
    },
    MuiAlert: {
      styleOverrides: {
        root: {
          borderRadius: 8,
          fontSize: "0.82rem",
          variants: [
            {
              props: { variant: "standard", color: "info" },
              style: {
                background: "var(--ui-rgba-57-208-255-060)",
                border: "1px solid var(--ui-rgba-57-208-255-150)",
              },
            },
            {
              props: { variant: "standard", color: "success" },
              style: {
                background: "var(--ui-rgba-74-210-157-060)",
                border: "1px solid var(--ui-rgba-74-210-157-150)",
              },
            },
            {
              props: { variant: "standard", color: "warning" },
              style: {
                background: "var(--ui-rgba-255-159-67-060)",
                border: "1px solid var(--ui-rgba-255-159-67-150)",
              },
            },
            {
              props: { variant: "standard", color: "error" },
              style: {
                background: "var(--ui-rgba-255-107-107-060)",
                border: "1px solid var(--ui-rgba-255-107-107-150)",
              },
            },
          ],
        },
      },
    },
    MuiTabs: {
      styleOverrides: {
        indicator: {
          backgroundColor: "var(--teal)",
          height: 2,
          borderRadius: 1,
          boxShadow: "0 0 8px var(--ui-rgba-0-255-170-150)",
        },
      },
    },
    MuiTab: {
      styleOverrides: {
        root: {
          textTransform: "none" as const,
          fontFamily: "var(--font-mono)",
          fontWeight: 500,
          fontSize: "0.78rem",
          letterSpacing: 0,
          color: "var(--text-secondary)",
          transition: "color 0.2s ease",
          "&:hover": {
            color: "var(--text-primary)",
          },
          "&.Mui-selected": {
            color: "var(--teal)",
            fontWeight: 600,
          },
        },
      },
    },
    MuiAccordion: {
      styleOverrides: {
        root: {
          background: "transparent",
          border: "1px solid var(--surface-border)",
          borderRadius: "8px !important",
          "&:before": {
            display: "none",
          },
        },
      },
    },
    MuiIconButton: {
      styleOverrides: {
        root: {
          padding: 6,
          borderRadius: "var(--button-radius)",
          border: "1px solid transparent",
          color: "var(--button-text-muted)",
          transition: "background 0.18s ease, border-color 0.18s ease, color 0.18s ease, box-shadow 0.18s ease, transform 0.18s ease",
          "&:hover": {
            background: "var(--button-bg-subtle-hover)",
            borderColor: "var(--button-border-strong)",
            color: "var(--button-text)",
            boxShadow: "none",
          },
          "&:active": {
            transform: "translateY(0) scale(0.98)",
            background: "var(--button-bg-pressed)",
          },
        },
      },
    },
    MuiToolbar: {
      styleOverrides: {
        regular: {
          minHeight: 50,
        },
      },
    },
    MuiTooltip: {
      styleOverrides: {
        tooltip: {
          background: "var(--cyber-panel-raised)",
          border: "1px solid var(--surface-border)",
          borderRadius: 8,
          color: "var(--text-primary)",
          fontFamily: "var(--font-mono)",
          fontSize: "0.72rem",
          backdropFilter: "blur(8px)",
        },
      },
    },
    MuiSwitch: {
      styleOverrides: {
        switchBase: {
          "&.Mui-checked": {
            color: "var(--teal)",
            "& + .MuiSwitch-track": {
              backgroundColor: "var(--ui-rgba-0-255-170-150)",
            },
          },
        },
      },
    },
  },
});
