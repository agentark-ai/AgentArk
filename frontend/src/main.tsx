import React from "react";
import ReactDOM from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { initializeUiSession } from "./api/client";
import { appTheme } from "./theme";
import "./styles.css";

const QUERY_STALE_TIME_MS = 10_000;
const QUERY_GC_TIME_MS = 2 * 60_000;

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: QUERY_STALE_TIME_MS,
      gcTime: QUERY_GC_TIME_MS,
      refetchOnWindowFocus: false,
      retry: 1
    }
  }
});

async function bootstrapAndRender() {
  try {
    await initializeUiSession();
  } catch {
    // Best-effort only. The shared API client still has fallback auth recovery.
  }

  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <QueryClientProvider client={queryClient}>
        <ThemeProvider theme={appTheme}>
          <CssBaseline />
          <App />
        </ThemeProvider>
      </QueryClientProvider>
    </React.StrictMode>
  );
}

void bootstrapAndRender();
