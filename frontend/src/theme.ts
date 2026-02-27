import { createTheme } from "@mui/material";

export const appTheme = createTheme({
  palette: {
    mode: "dark",
    primary: {
      main: "#2fd4ff"
    },
    secondary: {
      main: "#14f195"
    },
    background: {
      default: "#030711",
      paper: "#091527"
    },
    text: {
      primary: "#ecf5ff",
      secondary: "#9bb4d6"
    }
  },
  shape: {
    borderRadius: 12
  },
  typography: {
    fontFamily: "'Space Grotesk', 'IBM Plex Sans', 'Segoe UI', sans-serif",
    fontSize: 12,
    h4: {
      fontWeight: 700,
      fontSize: "1.3rem",
      lineHeight: 1.28
    },
    h3: {
      fontWeight: 700,
      fontSize: "1.45rem",
      lineHeight: 1.26
    },
    h5: {
      fontWeight: 600,
      fontSize: "1.12rem",
      lineHeight: 1.3
    },
    h6: {
      fontWeight: 600,
      fontSize: "0.98rem",
      lineHeight: 1.32
    },
    subtitle1: {
      fontSize: "0.9rem",
      lineHeight: 1.34
    },
    subtitle2: {
      fontSize: "0.82rem",
      lineHeight: 1.34
    },
    body1: {
      fontSize: "0.9rem",
      lineHeight: 1.42
    },
    body2: {
      fontSize: "0.84rem",
      lineHeight: 1.42
    },
    caption: {
      fontSize: "0.74rem",
      lineHeight: 1.34
    }
  },
  components: {
    MuiCard: {
      styleOverrides: {
        root: {
          border: "1px solid rgba(106, 150, 198, 0.22)",
          background: "linear-gradient(140deg, rgba(9,21,39,0.92), rgba(9,21,39,0.72))",
          backdropFilter: "blur(6px)"
        }
      }
    },
    MuiButton: {
      defaultProps: {
        size: "small"
      },
      styleOverrides: {
        root: {
          textTransform: "none",
          fontWeight: 600,
          fontSize: "0.75rem",
          paddingTop: 4,
          paddingBottom: 4
        }
      }
    },
    MuiTextField: {
      defaultProps: {
        size: "small"
      }
    },
    MuiIconButton: {
      styleOverrides: {
        root: {
          padding: 6
        }
      }
    },
    MuiToolbar: {
      styleOverrides: {
        regular: {
          minHeight: 52
        }
      }
    }
  }
});
