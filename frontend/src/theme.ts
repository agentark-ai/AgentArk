import { createTheme } from "@mui/material";

export const appTheme = createTheme({
  spacing: 8,
  palette: {
    mode: "dark",
    primary: {
      main: "#39d0ff",
      light: "#6cdbff",
      dark: "#2196f3",
    },
    secondary: {
      main: "#0fe3c2",
      light: "#4ad29d",
    },
    success: {
      main: "#4ad29d",
    },
    warning: {
      main: "#ff9f43",
    },
    error: {
      main: "#ff6b6b",
    },
    info: {
      main: "#39d0ff",
    },
    background: {
      default: "#02050f",
      paper: "#0a1220",
    },
    text: {
      primary: "#e8f4ff",
      secondary: "rgba(214, 228, 255, 0.6)",
    },
    divider: "rgba(64, 196, 255, 0.12)",
  },
  shape: {
    borderRadius: 12,
  },
  typography: {
    fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
    fontSize: 12,
    h3: {
      fontWeight: 700,
      fontSize: "1.5rem",
      lineHeight: 1.22,
      letterSpacing: "-0.01em",
    },
    h4: {
      fontWeight: 700,
      fontSize: "1.32rem",
      lineHeight: 1.24,
      letterSpacing: "-0.01em",
    },
    h5: {
      fontWeight: 700,
      fontSize: "1.14rem",
      lineHeight: 1.28,
      letterSpacing: "-0.015em",
    },
    h6: {
      fontWeight: 600,
      fontSize: "0.99rem",
      lineHeight: 1.32,
    },
    subtitle1: {
      fontSize: "0.91rem",
      fontWeight: 600,
      lineHeight: 1.36,
    },
    subtitle2: {
      fontSize: "0.83rem",
      fontWeight: 600,
      lineHeight: 1.38,
    },
    body1: {
      fontSize: "0.89rem",
      lineHeight: 1.5,
    },
    body2: {
      fontSize: "0.84rem",
      lineHeight: 1.48,
    },
    caption: {
      fontSize: "0.73rem",
      lineHeight: 1.38,
      color: "rgba(214, 228, 255, 0.6)",
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
          border: "1px solid rgba(64, 196, 255, 0.12)",
          background: "rgba(8, 18, 35, 0.88)",
          backdropFilter: "blur(10px)",
          borderRadius: 16,
          transition: "border-color 0.25s ease, box-shadow 0.25s ease, transform 0.2s ease",
          "&:hover": {
            borderColor: "rgba(64, 196, 255, 0.25)",
            boxShadow: "0 8px 24px rgba(57, 208, 255, 0.07)",
          },
        },
      },
    },
    MuiCardContent: {
      styleOverrides: {
        root: {
          padding: "14px",
          "&:last-child": {
            paddingBottom: "14px",
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
          fontSize: "0.76rem",
          lineHeight: 1.1,
          letterSpacing: "0.015em",
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
        },
        sizeSmall: {
          minHeight: "var(--button-height-sm)",
          padding: "0 var(--button-pad-x-sm)",
          fontSize: "0.76rem",
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
        containedSizeMedium: {
          minHeight: "var(--button-height-md)",
        },
        outlinedSizeMedium: {
          minHeight: "var(--button-height-md)",
        },
        textSizeMedium: {
          minHeight: "var(--button-height-md)",
        },
        contained: {
          background: "var(--button-bg-primary)",
          color: "var(--button-text-strong)",
          borderColor: "rgba(124, 230, 255, 0.24)",
          boxShadow: "var(--button-shadow-primary)",
          "&:hover": {
            background: "var(--button-bg-primary-hover)",
            borderColor: "rgba(141, 235, 255, 0.34)",
            boxShadow: "var(--button-shadow-hover)",
          },
        },
        containedPrimary: {
          background: "var(--button-bg-primary)",
          color: "var(--button-text-strong)",
          borderColor: "rgba(124, 230, 255, 0.24)",
          boxShadow: "var(--button-shadow-primary)",
          "&:hover": {
            background: "var(--button-bg-primary-hover)",
            borderColor: "rgba(141, 235, 255, 0.34)",
            boxShadow: "var(--button-shadow-hover)",
          },
        },
        containedSecondary: {
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
        containedSuccess: {
          color: "#07131f",
        },
        containedWarning: {
          color: "#07131f",
        },
        containedError: {
          color: "#f7fbff",
        },
        outlined: {
          background: "rgba(8, 18, 34, 0.48)",
          borderColor: "var(--button-border)",
          boxShadow: "none",
          "&:hover": {
            borderColor: "var(--button-border-strong)",
            background: "var(--button-bg-subtle-hover)",
            boxShadow: "var(--button-shadow)",
          },
        },
        outlinedPrimary: {
          color: "#8be7ff",
        },
        outlinedSuccess: {
          borderColor: "rgba(74, 210, 157, 0.28)",
          color: "#79f0bb",
          "&:hover": {
            borderColor: "rgba(74, 210, 157, 0.42)",
            background: "rgba(9, 37, 29, 0.82)",
          },
        },
        outlinedWarning: {
          borderColor: "rgba(255, 159, 67, 0.3)",
          color: "#ffbc7c",
          "&:hover": {
            borderColor: "rgba(255, 159, 67, 0.44)",
            background: "rgba(47, 24, 8, 0.82)",
          },
        },
        outlinedError: {
          borderColor: "rgba(255, 107, 107, 0.3)",
          color: "#ff9f9f",
          "&:hover": {
            borderColor: "rgba(255, 107, 107, 0.44)",
            background: "rgba(46, 11, 18, 0.82)",
          },
        },
        text: {
          color: "var(--button-text-muted)",
          borderColor: "transparent",
          background: "transparent",
          "&:hover": {
            color: "var(--button-text)",
            background: "rgba(12, 29, 51, 0.7)",
          },
        },
        textPrimary: {
          color: "var(--button-text)",
        },
        textError: {
          color: "#ffb0b0",
        },
        textWarning: {
          color: "#ffc98e",
        },
      },
    },
    MuiChip: {
      styleOverrides: {
        root: {
          fontFamily: "'JetBrains Mono', monospace",
          fontSize: "0.66rem",
          fontWeight: 500,
          letterSpacing: "0.03em",
          borderRadius: 999,
          transition: "all 0.2s ease",
          "&:hover": {
            boxShadow: "0 0 6px rgba(57, 208, 255, 0.15)",
          },
        },
        outlined: {
          borderColor: "rgba(64, 196, 255, 0.2)",
        },
        colorSuccess: {
          "&:hover": {
            boxShadow: "0 0 6px rgba(74, 210, 157, 0.25)",
          },
        },
        colorError: {
          "&:hover": {
            boxShadow: "0 0 6px rgba(255, 107, 107, 0.25)",
          },
        },
        colorWarning: {
          "&:hover": {
            boxShadow: "0 0 6px rgba(255, 159, 67, 0.25)",
          },
        },
      },
    },
    MuiTableContainer: {
      styleOverrides: {
        root: {
          borderRadius: 16,
          border: "1px solid rgba(64, 196, 255, 0.12)",
          background: "rgba(3, 9, 19, 0.5)",
        },
      },
    },
    MuiTableHead: {
      styleOverrides: {
        root: {
          "& .MuiTableCell-head": {
            background: "rgba(6, 14, 28, 0.8)",
            color: "rgba(214, 228, 255, 0.55)",
            fontWeight: 500,
            fontSize: "0.68rem",
            letterSpacing: "0.04em",
            textTransform: "uppercase" as const,
            borderBottom: "1px solid rgba(64, 196, 255, 0.12)",
            padding: "10px 14px",
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
            borderBottom: "1px solid rgba(64, 196, 255, 0.06)",
            padding: "10px 14px",
            fontSize: "0.8rem",
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
          fontSize: "0.78rem",
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
          borderRadius: 11,
          fontSize: "0.8rem",
          color: "#e8f4ff",
          "& .MuiOutlinedInput-notchedOutline": {
            borderColor: "rgba(64, 196, 255, 0.12)",
            transition: "border-color 0.2s ease, box-shadow 0.2s ease",
          },
          "&:hover .MuiOutlinedInput-notchedOutline": {
            borderColor: "rgba(64, 196, 255, 0.25)",
          },
          "&.Mui-focused .MuiOutlinedInput-notchedOutline": {
            borderColor: "#39d0ff",
            boxShadow: "0 0 0 2px rgba(57, 208, 255, 0.12), 0 0 8px rgba(57, 208, 255, 0.15)",
          },
        },
        input: {
          padding: "10px 12px",
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
          border: "1px solid rgba(64, 196, 255, 0.15)",
          borderRadius: 16,
          backdropFilter: "blur(14px)",
        },
      },
    },
    MuiAlert: {
      styleOverrides: {
        root: {
          borderRadius: 11,
          fontSize: "0.8rem",
        },
        standardInfo: {
          background: "rgba(57, 208, 255, 0.06)",
          border: "1px solid rgba(57, 208, 255, 0.15)",
        },
        standardSuccess: {
          background: "rgba(74, 210, 157, 0.06)",
          border: "1px solid rgba(74, 210, 157, 0.15)",
        },
        standardWarning: {
          background: "rgba(255, 159, 67, 0.06)",
          border: "1px solid rgba(255, 159, 67, 0.15)",
        },
        standardError: {
          background: "rgba(255, 107, 107, 0.06)",
          border: "1px solid rgba(255, 107, 107, 0.15)",
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
          border: "1px solid rgba(64, 196, 255, 0.12)",
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
            background: "rgba(12, 29, 51, 0.72)",
            borderColor: "rgba(93, 173, 236, 0.18)",
            color: "var(--button-text)",
            boxShadow: "var(--button-shadow)",
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
          minHeight: 54,
        },
      },
    },
    MuiTooltip: {
      styleOverrides: {
        tooltip: {
          background: "rgba(6, 14, 28, 0.95)",
          border: "1px solid rgba(57, 208, 255, 0.15)",
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
