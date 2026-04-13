import { createTheme } from "@mui/material";

export const appTheme = createTheme({
  spacing: 8,
  palette: {
    mode: "dark",
    primary: {
      main: "#4a7fae",
      light: "#6b97bf",
      dark: "#2d587f",
    },
    secondary: {
      main: "#8fa3c9",
      light: "#bcc8df",
    },
    success: {
      main: "#89d7ab",
    },
    warning: {
      main: "#ffbf82",
    },
    error: {
      main: "#ff9b9b",
    },
    info: {
      main: "#7ab8ff",
    },
    background: {
      default: "#0d0e11",
      paper: "#17181c",
    },
    text: {
      primary: "#f5f6f8",
      secondary: "rgba(213, 216, 223, 0.72)",
    },
    divider: "rgba(255, 255, 255, 0.08)",
  },
  shape: {
    borderRadius: 16,
  },
  typography: {
    fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
    fontSize: 14,
    h3: {
      fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 700,
      fontSize: "1.6rem",
      lineHeight: 1.24,
      letterSpacing: "-0.01em",
    },
    h4: {
      fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 700,
      fontSize: "1.4rem",
      lineHeight: 1.28,
      letterSpacing: "-0.01em",
    },
    h5: {
      fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 700,
      fontSize: "1.2rem",
      lineHeight: 1.32,
      letterSpacing: "-0.015em",
    },
    h6: {
      fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontWeight: 600,
      fontSize: "1rem",
      lineHeight: 1.34,
    },
    subtitle1: {
      fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.9rem",
      fontWeight: 600,
      lineHeight: 1.4,
    },
    subtitle2: {
      fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.82rem",
      fontWeight: 600,
      lineHeight: 1.4,
    },
    body1: {
      fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.94rem",
      lineHeight: 1.58,
    },
    body2: {
      fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.88rem",
      lineHeight: 1.54,
    },
    caption: {
      fontFamily: "'IBM Plex Sans', 'Segoe UI', sans-serif",
      fontSize: "0.72rem",
      lineHeight: 1.4,
      color: "rgba(213, 216, 223, 0.68)",
    },
  },
  components: {
    MuiCssBaseline: {
      styleOverrides: {
        body: {
          scrollbarWidth: "thin",
          scrollbarColor: "rgba(108,156,212,0.2) transparent",
        },
      },
    },
    MuiCard: {
      styleOverrides: {
        root: {
          border: "1px solid rgba(255, 255, 255, 0.08)",
          background: "linear-gradient(180deg, rgba(24, 24, 27, 0.94), rgba(17, 17, 20, 0.92))",
          backdropFilter: "blur(14px)",
          borderRadius: 20,
          transition: "border-color 0.25s ease, box-shadow 0.25s ease, transform 0.2s ease",
          "&:hover": {
            borderColor: "rgba(255, 255, 255, 0.12)",
            boxShadow: "0 12px 28px rgba(0, 0, 0, 0.18)",
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
          letterSpacing: "0.012em",
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
            color: "rgba(170, 193, 220, 0.38)",
            borderColor: "rgba(95, 132, 172, 0.14)",
            background: "rgba(10, 18, 31, 0.42)",
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
                borderColor: "rgba(74, 210, 157, 0.28)",
                color: "#79f0bb",
                "&:hover": {
                  borderColor: "rgba(74, 210, 157, 0.42)",
                  background: "rgba(9, 37, 29, 0.82)",
                },
              },
            },
            {
              props: { variant: "outlined", color: "warning" },
              style: {
                borderColor: "rgba(255, 159, 67, 0.3)",
                color: "#ffbc7c",
                "&:hover": {
                  borderColor: "rgba(255, 159, 67, 0.44)",
                  background: "rgba(47, 24, 8, 0.82)",
                },
              },
            },
            {
              props: { variant: "outlined", color: "error" },
              style: {
                borderColor: "rgba(255, 107, 107, 0.3)",
                color: "#ff9f9f",
                "&:hover": {
                  borderColor: "rgba(255, 107, 107, 0.44)",
                  background: "rgba(46, 11, 18, 0.82)",
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
          borderColor: "rgba(158, 185, 212, 0.24)",
          boxShadow: "var(--button-shadow-primary)",
          "&:hover": {
            background: "var(--button-bg-primary-hover)",
            borderColor: "rgba(173, 199, 224, 0.3)",
            boxShadow: "var(--button-shadow-hover)",
          },
        },
        outlined: {
          background: "rgba(22, 22, 26, 0.78)",
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
            background: "rgba(255, 255, 255, 0.05)",
          },
        },
      },
    },
    MuiChip: {
      styleOverrides: {
        root: {
          fontFamily: "'IBM Plex Sans', 'Space Grotesk', sans-serif",
          fontSize: "0.68rem",
          fontWeight: 500,
          letterSpacing: "0.025em",
          borderRadius: 999,
          transition: "all 0.2s ease",
          "&:hover": {
            boxShadow: "none",
          },
        },
        outlined: {
          borderColor: "rgba(118, 152, 190, 0.18)",
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
          borderRadius: 14,
          border: "1px solid rgba(118, 152, 190, 0.12)",
          background: "rgba(255, 255, 255, 0.018)",
        },
      },
    },
    MuiTableHead: {
      styleOverrides: {
        root: {
          "& .MuiTableCell-head": {
            background: "rgba(8, 18, 33, 0.74)",
            color: "rgba(214, 228, 255, 0.55)",
            fontWeight: 500,
            fontSize: "0.68rem",
            letterSpacing: "0.04em",
            textTransform: "uppercase" as const,
            borderBottom: "1px solid rgba(118, 152, 190, 0.12)",
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
              background: "rgba(57, 208, 255, 0.04) !important",
            },
          },
          "& .MuiTableCell-body": {
            borderBottom: "1px solid rgba(118, 152, 190, 0.06)",
            padding: "8px 12px",
            fontSize: "0.81rem",
            color: "#e8f4ff",
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
          color: "#e8f4ff !important",
        },
        input: {
          color: "#e8f4ff !important",
          "&::placeholder": {
            color: "rgba(140, 170, 210, 0.5) !important",
            opacity: "1 !important",
          },
        },
      },
    },
    MuiInputLabel: {
      styleOverrides: {
        root: {
          color: "rgba(214, 228, 255, 0.55)",
          "&.Mui-focused": {
            color: "#39d0ff",
          },
        },
      },
    },
    MuiSelect: {
      styleOverrides: {
        select: {
          color: "#e8f4ff",
        },
        icon: {
          color: "rgba(214, 228, 255, 0.5)",
        },
      },
    },
    MuiFormHelperText: {
      styleOverrides: {
        root: {
          color: "rgba(214, 228, 255, 0.45)",
        },
      },
    },
    MuiMenuItem: {
      styleOverrides: {
        root: {
          fontSize: "0.76rem",
          color: "#e8f4ff",
          "&:hover": {
            background: "rgba(57, 208, 255, 0.06)",
          },
          "&.Mui-selected": {
            background: "rgba(57, 208, 255, 0.1)",
            "&:hover": {
              background: "rgba(57, 208, 255, 0.14)",
            },
          },
        },
      },
    },
    MuiOutlinedInput: {
      styleOverrides: {
        root: {
          borderRadius: 10,
          fontSize: "0.82rem",
          color: "#e8f4ff",
          "& .MuiOutlinedInput-notchedOutline": {
            borderColor: "rgba(118, 152, 190, 0.12)",
            transition: "border-color 0.2s ease, box-shadow 0.2s ease",
          },
          "&:hover .MuiOutlinedInput-notchedOutline": {
            borderColor: "rgba(118, 152, 190, 0.2)",
          },
          "&.Mui-focused .MuiOutlinedInput-notchedOutline": {
            borderColor: "#39d0ff",
            boxShadow: "0 0 0 2px rgba(57, 208, 255, 0.1)",
          },
        },
        input: {
          padding: "9px 11px",
          color: "#e8f4ff",
          "&::placeholder": {
            color: "rgba(140, 170, 210, 0.4)",
            opacity: 1,
          },
        },
      },
    },
    MuiDialog: {
      styleOverrides: {
        paper: {
          background: "#0a1220",
          border: "1px solid rgba(118, 152, 190, 0.14)",
          borderRadius: 16,
          backdropFilter: "blur(14px)",
        },
      },
    },
    MuiAlert: {
      styleOverrides: {
        root: {
          borderRadius: 10,
          fontSize: "0.82rem",
          variants: [
            {
              props: { variant: "standard", color: "info" },
              style: {
                background: "rgba(57, 208, 255, 0.06)",
                border: "1px solid rgba(57, 208, 255, 0.15)",
              },
            },
            {
              props: { variant: "standard", color: "success" },
              style: {
                background: "rgba(74, 210, 157, 0.06)",
                border: "1px solid rgba(74, 210, 157, 0.15)",
              },
            },
            {
              props: { variant: "standard", color: "warning" },
              style: {
                background: "rgba(255, 159, 67, 0.06)",
                border: "1px solid rgba(255, 159, 67, 0.15)",
              },
            },
            {
              props: { variant: "standard", color: "error" },
              style: {
                background: "rgba(255, 107, 107, 0.06)",
                border: "1px solid rgba(255, 107, 107, 0.15)",
              },
            },
          ],
        },
      },
    },
    MuiTabs: {
      styleOverrides: {
        indicator: {
          backgroundColor: "#39d0ff",
          height: 2,
          borderRadius: 1,
          boxShadow: "0 0 8px rgba(57, 208, 255, 0.3)",
        },
      },
    },
    MuiTab: {
      styleOverrides: {
        root: {
          textTransform: "none" as const,
          fontWeight: 500,
          fontSize: "0.78rem",
          letterSpacing: "0.01em",
          color: "rgba(214, 228, 255, 0.55)",
          transition: "color 0.2s ease",
          "&:hover": {
            color: "rgba(214, 228, 255, 0.85)",
          },
          "&.Mui-selected": {
            color: "#e8f4ff",
            fontWeight: 600,
          },
        },
      },
    },
    MuiAccordion: {
      styleOverrides: {
        root: {
          background: "transparent",
          border: "1px solid rgba(118, 152, 190, 0.12)",
          borderRadius: "12px !important",
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
          background: "rgba(6, 14, 28, 0.95)",
          border: "1px solid rgba(118, 152, 190, 0.14)",
          borderRadius: 8,
          fontSize: "0.72rem",
          backdropFilter: "blur(8px)",
        },
      },
    },
    MuiSwitch: {
      styleOverrides: {
        switchBase: {
          "&.Mui-checked": {
            color: "#39d0ff",
            "& + .MuiSwitch-track": {
              backgroundColor: "rgba(57, 208, 255, 0.35)",
            },
          },
        },
      },
    },
  },
});
