import {
  Component,
  type CSSProperties,
  type ErrorInfo,
  type ReactNode
} from "react";

interface RootErrorBoundaryProps {
  children: ReactNode;
}

interface RootErrorBoundaryState {
  error: Error | null;
}

const panelStyles: Record<string, CSSProperties> = {
  root: {
    position: "fixed",
    inset: 0,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    padding: 24,
    background: "#0b0f0c",
    color: "#eef6ef",
    fontFamily: "Inter, 'Segoe UI', system-ui, sans-serif",
    zIndex: 2147483647
  },
  panel: {
    width: "min(460px, calc(100vw - 48px))",
    boxSizing: "border-box",
    border: "1px solid rgba(0, 255, 170, 0.2)",
    borderRadius: 10,
    background: "#111611",
    boxShadow: "0 22px 70px rgba(0,0,0,.55), inset 0 1px 0 rgba(255,255,255,.05)",
    padding: 24
  },
  title: {
    margin: "0 0 10px",
    color: "#f5fff8",
    font: "700 15px/1.3 'JetBrains Mono', Consolas, monospace"
  },
  body: {
    margin: "0 0 8px",
    fontSize: 14,
    lineHeight: 1.5,
    color: "rgba(238,246,239,.86)"
  },
  detail: {
    margin: "0 0 18px",
    fontSize: 12,
    lineHeight: 1.45,
    color: "rgba(238,246,239,.55)",
    whiteSpace: "pre-wrap",
    overflowWrap: "anywhere",
    maxHeight: 96,
    overflow: "auto",
    fontFamily: "'JetBrains Mono', Consolas, monospace"
  },
  actions: {
    display: "flex",
    justifyContent: "flex-end"
  },
  reload: {
    minWidth: 96,
    minHeight: 36,
    border: "1px solid rgba(0,255,170,.24)",
    borderRadius: 8,
    background: "#16d6d8",
    color: "#031112",
    font: "700 13px/1 Inter, 'Segoe UI', system-ui, sans-serif",
    cursor: "pointer",
    padding: "0 16px"
  }
};

/**
 * Last-resort boundary above all providers. Without it, any render error
 * anywhere in the tree unmounts the React root and blanks the page (the
 * tree below is Suspense-only). Kept free of MUI/theme imports so it can
 * still render when the theme or component library itself is what broke.
 */
export class RootErrorBoundary extends Component<
  RootErrorBoundaryProps,
  RootErrorBoundaryState
> {
  state: RootErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): RootErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("[root-error-boundary]", error, info.componentStack);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;
    return (
      <div style={panelStyles.root} role="alert">
        <div style={panelStyles.panel}>
          <h1 style={panelStyles.title}>AgentArk hit a rendering error</h1>
          <p style={panelStyles.body}>
            The page stopped, not the agent — any run in progress continues in
            the background and will be picked up after reload.
          </p>
          <pre style={panelStyles.detail}>
            {error.message || String(error)}
          </pre>
          <div style={panelStyles.actions}>
            <button
              type="button"
              style={panelStyles.reload}
              onClick={() => window.location.reload()}
            >
              Reload
            </button>
          </div>
        </div>
      </div>
    );
  }
}
