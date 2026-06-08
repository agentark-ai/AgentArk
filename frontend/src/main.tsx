import React from "react";
import ReactDOM from "react-dom/client";
import { CssBaseline, ThemeProvider } from "@mui/material";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import App from "./App";
import { initializeUiSession } from "./api/client";
import { RootErrorBoundary } from "./components/RootErrorBoundary";
import { appTheme } from "./theme";
import "./fonts.css";
import "./styles.css";

const QUERY_STALE_TIME_MS = 10_000;
const QUERY_GC_TIME_MS = 2 * 60_000;

// After the server is rebuilt while a tab stays open, the tab's chunk graph
// goes stale: a lazy panel (or the background settings warmup) can request a
// fingerprinted asset that no longer exists, the dynamic import rejects past
// the Suspense-only tree, and the whole page unmounts. Vite surfaces that
// failure as `vite:preloadError` for DYNAMICALLY imported chunks — reload
// once to pick up the fresh index.html instead of crashing. Scope note: this
// cannot cover static entry/vendor imports (main.tsx never runs if those
// fail), but those are only fetched at page load against a no-store
// index.html, so they cannot go stale in an open tab. The session flag
// (cleared after the reloaded app has stayed up) prevents a reload loop when
// an asset is genuinely missing rather than merely stale.
const CHUNK_RELOAD_FLAG = "agentark:chunk-reload";
window.addEventListener("vite:preloadError", (event) => {
  if (sessionStorage.getItem(CHUNK_RELOAD_FLAG) === "1") return;
  sessionStorage.setItem(CHUNK_RELOAD_FLAG, "1");
  event.preventDefault();
  window.location.reload();
});

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
      <RootErrorBoundary>
        <QueryClientProvider client={queryClient}>
          <ThemeProvider theme={appTheme}>
            <CssBaseline />
            <App />
          </ThemeProvider>
        </QueryClientProvider>
      </RootErrorBoundary>
    </React.StrictMode>
  );

  // The app survived the post-reload window: re-arm the stale-chunk heal so a
  // future rebuild in this same tab session can recover again.
  window.setTimeout(() => sessionStorage.removeItem(CHUNK_RELOAD_FLAG), 30_000);
}

void bootstrapAndRender();
