import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent,
} from "react";
import { Alert, Box, IconButton, Stack, Tooltip, Typography } from "@mui/material";
import OpenWithRoundedIcon from "@mui/icons-material/OpenWithRounded";
import AgentLogo from "../../assets/logo.svg";
import { arkorbitApi } from "../arkorbit/api";
import { OrbitChat } from "../arkorbit/OrbitChat";
import { OrbitFrame } from "../arkorbit/OrbitFrame";
import { OrbitSettingsDialog } from "../arkorbit/OrbitSettingsDialog";
import { OrbitSwitcher } from "../arkorbit/OrbitSwitcher";
import type { Orbit, OrbitId } from "../arkorbit/types";

type OrbitHomeDashboardProps = {
  orbits: Orbit[];
  activeOrbitId: OrbitId | null;
  onSelect: (id: OrbitId) => void;
};

type ChatAnchor = { x: number; y: number };
type DragState = {
  pointerId: number;
  startX: number;
  startY: number;
  originX: number;
  originY: number;
  width: number;
  height: number;
};

function OrbitHomeDashboard({
  orbits,
  activeOrbitId,
  onSelect,
}: OrbitHomeDashboardProps) {
  return (
    <Box className="orbit-home-canvas">
      <Box className="orbit-home-panel">
        <Box className="orbit-system-banner">
          <span className="orbit-system-banner-eyebrow">System // Orbit Runtime</span>
          <div className="orbit-system-banner-title">A live workspace the agent builds for you</div>
          <div className="orbit-system-banner-body">
            Orbit is a browser canvas the agent reshapes on demand. Ask for a widget, dashboard, or tool and the agent assembles modules right onto this surface. Each canvas is its own filesystem, and changes persist across sessions so you can keep building.
          </div>
          <div className="orbit-system-banner-pills">
            <span className="orbit-system-banner-pill">
              <span className="orbit-system-banner-pill-dot" /> Canvases
            </span>
            <span className="orbit-system-banner-pill">
              <span className="orbit-system-banner-pill-dot" /> Agent-deployed
            </span>
            <span className="orbit-system-banner-pill">
              <span className="orbit-system-banner-pill-dot" /> Persistent
            </span>
          </div>
        </Box>
        <Stack spacing={0.5} className="orbit-home-heading">
          <Typography variant="overline">Orbit canvases</Typography>
          <Typography variant="h4">Home</Typography>
        </Stack>
        <Box className="orbit-home-grid">
          {orbits.map((orbit) => {
            const active = orbit.id === activeOrbitId;
            return (
              <button
                key={orbit.id}
                type="button"
                className={`orbit-home-card${active ? " is-active" : ""}`}
                onClick={() => onSelect(orbit.id)}
              >
                <span
                  className="orbit-home-card-accent"
                  style={orbit.color ? { background: orbit.color } : undefined}
                />
                <span className="orbit-home-card-title">{orbit.name}</span>
                <span className="orbit-home-card-meta">
                  {orbit.is_default ? "Home dashboard" : "Canvas"}
                </span>
              </button>
            );
          })}
        </Box>
      </Box>
    </Box>
  );
}

export function ArkOrbitPage() {
  const [orbits, setOrbits] = useState<Orbit[]>([]);
  const [activeOrbitId, setActiveOrbitId] = useState<OrbitId | null>(null);
  const [settingsOrbitId, setSettingsOrbitId] = useState<OrbitId | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [bootstrapping, setBootstrapping] = useState(true);
  const [frameVersion, setFrameVersion] = useState(0);
  const [orbitReloadSignal, setOrbitReloadSignal] = useState(0);
  const [chatOpen, setChatOpen] = useState(false);
  const [chatAnchor, setChatAnchor] = useState<ChatAnchor | null>(null);
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const chatDragRef = useRef<DragState | null>(null);
  const chatMovedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await arkorbitApi.listOrbits();
        if (cancelled) return;
        setOrbits(result.orbits);
        setActiveOrbitId(result.orbits[0]?.id ?? null);
      } catch (err) {
        if (!cancelled) {
          setLoadError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (!cancelled) setBootstrapping(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const activeOrbit = useMemo(
    () => orbits.find((orbit) => orbit.id === activeOrbitId) ?? null,
    [activeOrbitId, orbits],
  );
  const showHomeDashboard = Boolean(activeOrbit?.is_default);

  const handleSelectOrbit = useCallback((id: OrbitId) => {
    setActiveOrbitId(id);
    setFrameVersion((prev) => prev + 1);
  }, []);

  const handleOrbitCreated = useCallback((orbit: Orbit) => {
    setOrbits((prev) => [...prev, orbit]);
    setActiveOrbitId(orbit.id);
    setFrameVersion((prev) => prev + 1);
    setChatOpen(true);
  }, []);

  const handleOrbitUpdated = useCallback((orbit: Orbit) => {
    setOrbits((prev) => prev.map((item) => (item.id === orbit.id ? orbit : item)));
  }, []);

  const handleOrbitDeleted = useCallback(
    (id: OrbitId) => {
      setOrbits((prev) => {
        const remaining = prev.filter((orbit) => orbit.id !== id);
        if (id === activeOrbitId) {
          setActiveOrbitId(remaining[0]?.id ?? null);
          setFrameVersion((version) => version + 1);
        }
        return remaining;
      });
    },
    [activeOrbitId],
  );

  const clampChatAnchor = useCallback((x: number, y: number, width = 54, height = 54): ChatAnchor => {
    const rect = canvasRef.current?.getBoundingClientRect();
    const canvasWidth = rect?.width ?? 1200;
    const canvasHeight = rect?.height ?? 800;
    return {
      x: Math.min(Math.max(10, x), Math.max(10, canvasWidth - width - 10)),
      y: Math.min(Math.max(10, y), Math.max(10, canvasHeight - height - 10)),
    };
  }, []);

  const chatFloatingStyle = useMemo(() => {
    if (!chatAnchor) return undefined;
    return {
      left: `${chatAnchor.x}px`,
      top: `${chatAnchor.y}px`,
      right: "auto",
      bottom: "auto",
    } satisfies CSSProperties;
  }, [chatAnchor]);

  const chatFlyoutStyle = useMemo(() => {
    if (!chatAnchor) return undefined;
    const rect = canvasRef.current?.getBoundingClientRect();
    const width = rect?.width ?? 1200;
    const height = rect?.height ?? 800;
    const panelWidth = Math.min(430, Math.max(280, width - 44));
    const panelHeight = Math.min(680, Math.max(360, height - 36));
    return {
      left: `${Math.min(chatAnchor.x, Math.max(18, width - panelWidth - 18))}px`,
      top: `${Math.min(chatAnchor.y, Math.max(18, height - panelHeight - 18))}px`,
      right: "auto",
      bottom: "auto",
      width: `${panelWidth}px`,
      height: `${panelHeight}px`,
    } satisfies CSSProperties;
  }, [chatAnchor, chatOpen]);

  const handleChatPointerDown = useCallback(
    (event: PointerEvent<HTMLButtonElement>) => {
      if (event.button !== 0) return;
      const rect = canvasRef.current?.getBoundingClientRect();
      const buttonRect = event.currentTarget.getBoundingClientRect();
      const origin = chatAnchor ?? {
        x: rect ? buttonRect.left - rect.left : buttonRect.left,
        y: rect ? buttonRect.top - rect.top : buttonRect.top,
      };
      chatDragRef.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        originX: origin.x,
        originY: origin.y,
        width: buttonRect.width,
        height: buttonRect.height,
      };
      chatMovedRef.current = false;
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [chatAnchor],
  );

  const handleChatPointerMove = useCallback(
    (event: PointerEvent<HTMLElement>) => {
      const drag = chatDragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      const dx = event.clientX - drag.startX;
      const dy = event.clientY - drag.startY;
      if (Math.abs(dx) > 3 || Math.abs(dy) > 3) {
        chatMovedRef.current = true;
      }
      setChatAnchor(
        clampChatAnchor(drag.originX + dx, drag.originY + dy, drag.width, drag.height),
      );
    },
    [clampChatAnchor],
  );

  const handleChatPointerUp = useCallback((event: PointerEvent<HTMLElement>) => {
    const drag = chatDragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    chatDragRef.current = null;
    try {
      event.currentTarget.releasePointerCapture(event.pointerId);
    } catch {
      // Pointer capture may already be gone after browser gestures.
    }
  }, []);

  const handleChatToggleClick = useCallback(() => {
    if (chatMovedRef.current) {
      chatMovedRef.current = false;
      return;
    }
    setChatOpen((open) => !open);
  }, []);

  const handleChatFlyoutPointerDown = useCallback((event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    const target = event.target;
    const dragHandle =
      target instanceof Element ? target.closest("[data-orbit-chat-drag-handle]") : null;
    if (!dragHandle) return;
    const rect = canvasRef.current?.getBoundingClientRect();
    const flyoutRect = event.currentTarget.getBoundingClientRect();
    chatDragRef.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      originX: rect ? flyoutRect.left - rect.left : flyoutRect.left,
      originY: rect ? flyoutRect.top - rect.top : flyoutRect.top,
      width: flyoutRect.width,
      height: flyoutRect.height,
    };
    chatMovedRef.current = false;
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }, []);

  return (
    <Box className="arkorbit-shell">
      <Box className="arkorbit-header">
        <Box className="arkorbit-heading">
          <img src={AgentLogo} alt="AgentArk" className="arkorbit-agent-logo" />
          <Stack sx={{ minWidth: 0 }}>
            <Typography variant="h6" className="arkorbit-title">
              Orbit
            </Typography>
            <Typography variant="caption" className="arkorbit-subtitle">
              {activeOrbit ? activeOrbit.name : bootstrapping ? "Loading..." : "No orbit"}
            </Typography>
          </Stack>
        </Box>
        <OrbitSwitcher
          orbits={orbits}
          activeOrbitId={activeOrbitId}
          onSelect={handleSelectOrbit}
          onCreated={handleOrbitCreated}
          onOpenSettings={setSettingsOrbitId}
        />
      </Box>
      {loadError ? (
        <Alert
          severity="warning"
          className="arkorbit-error"
          onClose={() => setLoadError(null)}
        >
          {loadError}
        </Alert>
      ) : null}
      <Box className="arkorbit-canvas-wrap" ref={canvasRef}>
        <Box className="arkorbit-frame-pane">
          {activeOrbitId && showHomeDashboard ? (
            <OrbitHomeDashboard
              orbits={orbits}
              activeOrbitId={activeOrbitId}
              onSelect={handleSelectOrbit}
            />
          ) : activeOrbitId ? (
            <OrbitFrame
              key={`${activeOrbitId}:${frameVersion}`}
              orbitId={activeOrbitId}
              externalReloadToken={orbitReloadSignal}
              onRuntimeNotice={setLoadError}
            />
          ) : (
            <Box className="orbit-home-canvas">
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                {bootstrapping ? "Loading orbit..." : "No orbit available."}
              </Typography>
            </Box>
          )}
        </Box>
        {activeOrbitId && !chatOpen ? (
          <Tooltip title="Open Orbit chat">
            <IconButton
              className="arkorbit-chat-bubble"
              style={chatFloatingStyle}
              onPointerDown={handleChatPointerDown}
              onPointerMove={handleChatPointerMove}
              onPointerUp={handleChatPointerUp}
              onPointerCancel={handleChatPointerUp}
              onClick={handleChatToggleClick}
              aria-label="Open Orbit chat"
            >
              <img src={AgentLogo} alt="" className="arkorbit-chat-bubble-logo" />
            </IconButton>
          </Tooltip>
        ) : null}
        {activeOrbitId && chatOpen ? (
          <Box
            className={`arkorbit-chat-flyout${chatAnchor ? " is-positioned" : ""}`}
            style={chatFlyoutStyle}
            onPointerDown={handleChatFlyoutPointerDown}
            onPointerMove={handleChatPointerMove}
            onPointerUp={handleChatPointerUp}
            onPointerCancel={handleChatPointerUp}
          >
            <Tooltip title="Move chat">
              <IconButton
                size="small"
                className="arkorbit-chat-drag-handle"
                data-orbit-chat-drag-handle="true"
                aria-label="Move Orbit chat"
              >
                <OpenWithRoundedIcon fontSize="small" />
              </IconButton>
            </Tooltip>
            <OrbitChat
              orbitId={activeOrbitId}
              onFileWritten={() => setOrbitReloadSignal((version) => version + 1)}
              onClose={() => setChatOpen(false)}
            />
          </Box>
        ) : null}
      </Box>
      <OrbitSettingsDialog
        orbitId={settingsOrbitId}
        orbits={orbits}
        open={settingsOrbitId !== null}
        onClose={() => setSettingsOrbitId(null)}
        onUpdated={handleOrbitUpdated}
        onDeleted={handleOrbitDeleted}
      />
    </Box>
  );
}

export default ArkOrbitPage;
