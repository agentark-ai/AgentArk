import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ChangeEvent,
  type FormEvent,
  type PointerEvent,
} from "react";
import {
  Alert,
  Box,
  Dialog,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import ChevronLeftRoundedIcon from "@mui/icons-material/ChevronLeftRounded";
import ChevronRightRoundedIcon from "@mui/icons-material/ChevronRightRounded";
import OpenWithRoundedIcon from "@mui/icons-material/OpenWithRounded";
import AgentLogo from "../../assets/logo.svg";
import { arkorbitApi, type CreateOrbitPayload } from "../arkorbit/api";
import { OrbitChat } from "../arkorbit/OrbitChat";
import { OrbitFrame } from "../arkorbit/OrbitFrame";
import { OrbitSettingsDialog } from "../arkorbit/OrbitSettingsDialog";
import { OrbitSwitcher } from "../arkorbit/OrbitSwitcher";
import type { Orbit, OrbitId } from "../arkorbit/types";

type OrbitHomeDashboardProps = {
  orbits: Orbit[];
  onSelect: (id: OrbitId) => void;
  onCreateSubmit?: (payload: CreateOrbitPayload) => void;
};

const LEGACY_CANVAS_EXAMPLES = [
  {
    cat: "Systems",
    text: "A dashboard of my background agents — name, last run, success rate.",
    emphasis: "dashboard of my background agents",
  },
  {
    cat: "Calm",
    text: "A breathing widget, a quote of the day, and a do-not-disturb timer.",
    emphasis: "do-not-disturb",
  },
];

const HOME_ORBITS_PER_PAGE = 10;

type ChatAnchor = { x: number; y: number };
type OrbitRouteState = {
  orbitId: OrbitId | null;
  chatOpen: boolean;
};
type DragState = {
  pointerId: number;
  startX: number;
  startY: number;
  originX: number;
  originY: number;
  width: number;
  height: number;
};

const MAX_RUNTIME_NOTICE_USES = 2;

function readOrbitRouteState(): OrbitRouteState {
  if (typeof window === "undefined") {
    return { orbitId: null, chatOpen: false };
  }
  try {
    const params = new URLSearchParams(window.location.search);
    const orbitId = (params.get("orbit") || params.get("canvas") || "").trim() || null;
    const chatValue = (params.get("chat") || "").trim().toLocaleLowerCase();
    return {
      orbitId,
      chatOpen: chatValue === "1" || chatValue === "true" || chatValue === "open",
    };
  } catch {
    return { orbitId: null, chatOpen: false };
  }
}

function writeOrbitRouteState(orbit: Orbit | null, chatOpen: boolean) {
  if (typeof window === "undefined") return;
  const normalizedPath = window.location.pathname.replace(/\/+$/, "");
  if (!normalizedPath.startsWith("/ui/arkorbit")) return;

  const params = new URLSearchParams(window.location.search);
  params.delete("canvas");
  if (orbit && !orbit.is_default) {
    params.set("orbit", orbit.id);
    if (chatOpen) {
      params.set("chat", "1");
    } else {
      params.delete("chat");
    }
  } else {
    params.delete("orbit");
    params.delete("chat");
  }

  const nextSearch = params.toString();
  const nextUrl = `/ui/arkorbit${nextSearch ? `?${nextSearch}` : ""}${window.location.hash}`;
  const currentUrl = `${window.location.pathname}${window.location.search}${window.location.hash}`;
  if (nextUrl !== currentUrl) {
    window.history.replaceState(null, "", nextUrl);
  }
}

function normalizeRuntimeNoticeForBudget(message: string): string {
  return message
    .trim()
    .toLocaleLowerCase()
    .replace(/\s+/g, " ")
    .slice(0, 220);
}

function OrbitHomeDashboard({
  orbits,
  onSelect,
  onCreateSubmit,
}: OrbitHomeDashboardProps) {
  const canvasOrbits = useMemo(
    () => orbits.filter((orbit) => !orbit.is_default),
    [orbits],
  );
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("");
  const [color, setColor] = useState("#78f2b0");
  const [nameError, setNameError] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [orbitPage, setOrbitPage] = useState(0);
  const [fileCounts, setFileCounts] = useState<Record<OrbitId, number>>({});
  const [widgetCounts, setWidgetCounts] = useState<Record<OrbitId, number>>({});
  const nameInputRef = useRef<HTMLInputElement | null>(null);
  const latestCanvasOrbits = useMemo(
    () =>
      canvasOrbits
        .map((orbit, index) => {
          const parsed = Date.parse(orbit.updated_at || orbit.created_at || "");
          return {
            orbit,
            index,
            sortTime: Number.isFinite(parsed) ? parsed : 0,
          };
        })
        .sort((a, b) => b.sortTime - a.sortTime || a.index - b.index)
        .map((item) => item.orbit),
    [canvasOrbits],
  );
  const orbitPageCount = Math.max(
    1,
    Math.ceil(latestCanvasOrbits.length / HOME_ORBITS_PER_PAGE),
  );
  const effectiveOrbitPage = Math.min(orbitPage, orbitPageCount - 1);
  const visibleOrbitStart = effectiveOrbitPage * HOME_ORBITS_PER_PAGE;
  const visibleCanvasOrbits = useMemo(
    () =>
      latestCanvasOrbits.slice(
        visibleOrbitStart,
        visibleOrbitStart + HOME_ORBITS_PER_PAGE,
      ),
    [latestCanvasOrbits, visibleOrbitStart],
  );
  const visibleOrbitEnd =
    visibleCanvasOrbits.length > 0
      ? visibleOrbitStart + visibleCanvasOrbits.length
      : 0;

  useEffect(() => {
    setOrbitPage((page) => Math.min(page, orbitPageCount - 1));
  }, [orbitPageCount]);

  useEffect(() => {
    if (!creating) return;
    const timer = window.setTimeout(() => {
      nameInputRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [creating]);

  useEffect(() => {
    if (visibleCanvasOrbits.length === 0) return;
    let cancelled = false;
    Promise.all(
      visibleCanvasOrbits.map(async (orbit) => {
        try {
          const files = await arkorbitApi.listFiles(orbit.id);
          const widgets = files.filter(
            (f) =>
              /^mod\/[^/]+\/index\.(js|mjs|ts|tsx|jsx|html)$/i.test(f.path) ||
              /^widgets\/[^/]+\/index\./i.test(f.path),
          ).length;
          return { id: orbit.id, total: files.length, widgets };
        } catch {
          return { id: orbit.id, total: 0, widgets: 0 };
        }
      }),
    ).then((rows) => {
      if (cancelled) return;
      const fc: Record<OrbitId, number> = {};
      const wc: Record<OrbitId, number> = {};
      for (const r of rows) {
        fc[r.id] = r.total;
        wc[r.id] = r.widgets;
      }
      setFileCounts(fc);
      setWidgetCounts(wc);
    });
    return () => {
      cancelled = true;
    };
  }, [visibleCanvasOrbits]);

  const submit = useCallback(() => {
    const trimmedName = name.trim();
    if (!trimmedName) {
      setNameError("Name is required.");
      return;
    }
    if (
      orbits.some(
        (orbit) => orbit.name.trim().toLocaleLowerCase() === trimmedName.toLocaleLowerCase(),
      )
    ) {
      setNameError("A canvas with this name already exists.");
      return;
    }
    if (onCreateSubmit) {
      const payload: CreateOrbitPayload = { name: trimmedName };
      const trimmedIcon = icon.trim();
      if (trimmedIcon) payload.icon = trimmedIcon;
      if (color.trim()) payload.color = color.trim();
      onCreateSubmit(payload);
    }
    setName("");
    setIcon("");
    setColor("#78f2b0");
    setNameError(null);
    setCreating(false);
  }, [color, icon, name, onCreateSubmit, orbits]);

  const duplicateName = orbits.some(
    (orbit) => orbit.name.trim().toLocaleLowerCase() === name.trim().toLocaleLowerCase(),
  );
  const canSubmit = name.trim().length > 0 && !duplicateName;
  const startCreating = () => setCreating(true);
  const cancelCreating = () => {
    setCreating(false);
    setName("");
    setIcon("");
    setColor("#78f2b0");
    setNameError(null);
  };

  const monoFont = "'JetBrains Mono', 'IBM Plex Mono', Menlo, monospace";
  const serifDisplay = "'Playfair Display', Georgia, serif";
  const serifBody = "'EB Garamond', Georgia, serif";
  const accent = "#78f2b0";
  const warm = "#e8b46d";
  const ink = "#e8eef5";
  const inkSoft = "rgba(232,238,245,0.7)";
  const inkDim = "rgba(232,238,245,0.4)";
  const ruleSoft = "rgba(216,195,152,0.14)";
  const panelTint = "rgba(13, 17, 23, 0.62)";

  const formatRelative = (iso?: string): string => {
    if (!iso) return "—";
    const ts = Date.parse(iso);
    if (!Number.isFinite(ts)) return "—";
    const diff = Date.now() - ts;
    const m = Math.round(diff / 60000);
    if (m < 1) return "just now";
    if (m < 60) return `${m}m ago`;
    const h = Math.round(m / 60);
    if (h < 24) return `${h}h ago`;
    const d = Math.round(h / 24);
    if (d < 7) return `${d}d ago`;
    if (d < 30) return `${Math.round(d / 7)}w ago`;
    if (d < 365) return `${Math.round(d / 30)}mo ago`;
    return `${Math.round(d / 365)}y ago`;
  };

  const accentForOrbit = (orbit: Orbit, idx: number): string => {
    if (orbit.color) return orbit.color;
    const palette = ["#78f2b0", "#b7a7ff", "#6dc497", "#e8b46d", "#d96d83", "#c8d8c9", "#e6d6c0", "#fda4af"];
    return palette[idx % palette.length];
  };

  const lastTouched = latestCanvasOrbits[0]
    ? formatRelative(latestCanvasOrbits[0].updated_at || latestCanvasOrbits[0].created_at)
    : "—";

  return (
    <Box
      sx={{
        position: "relative",
        minHeight: "100%",
        color: ink,
        background:
          "radial-gradient(ellipse 60% 50% at 20% 10%, rgba(120,242,176,0.05), transparent 60%)," +
          "radial-gradient(ellipse 50% 50% at 90% 90%, rgba(232,180,109,0.03), transparent 65%)," +
          "linear-gradient(180deg, rgba(6,8,11,1), rgba(5,6,8,1))",
        pb: 6,
      }}
    >
      <Box
        sx={{
          maxWidth: 1280,
          mx: "auto",
          px: { xs: 2, sm: 3, md: 4 },
          pt: { xs: 4, md: 6 },
          pb: 2.5,
          display: "flex",
          justifyContent: "space-between",
          alignItems: "flex-end",
          gap: 2,
          flexWrap: "wrap",
        }}
      >
        <Box sx={{ minWidth: 0 }}>
          <Typography sx={{ fontFamily: monoFont, fontSize: 11, letterSpacing: "0.22em", color: accent, textTransform: "uppercase", fontWeight: 700, mb: 1.4 }}>
            ▸ ArkOrbit · Your canvases
          </Typography>
          <Typography sx={{ fontSize: { xs: 26, md: 32 }, fontWeight: 800, letterSpacing: "-0.02em", lineHeight: 1.05, mb: 0.6, color: ink }}>
            {canvasOrbits.length === 0 ? "Create your first canvas." : `${canvasOrbits.length} ${canvasOrbits.length === 1 ? "canvas" : "canvases"}.`}
          </Typography>
          <Typography sx={{ fontSize: 14, color: inkSoft, m: 0 }}>
            {latestCanvasOrbits[0]
              ? <>Last touched <Box component="strong" sx={{ color: ink }}>{latestCanvasOrbits[0].name}</Box> · {lastTouched}.</>
              : "Name a canvas and choose its accent color."}
          </Typography>
        </Box>
        <Box
          component="button"
          type="button"
          onClick={startCreating}
          sx={{
            px: 2.4, py: 1.2,
            background: "rgba(120,242,176,0.1)",
            border: "1px solid rgba(120,242,176,0.45)",
            borderRadius: 1,
            color: accent,
            fontFamily: monoFont, fontSize: 11.5, letterSpacing: "0.16em", textTransform: "uppercase", fontWeight: 700,
            cursor: "pointer", flexShrink: 0,
            transition: "all 160ms ease",
            "&:hover": { background: "rgba(120,242,176,0.18)" },
          }}
        >
          + New Canvas
        </Box>
      </Box>

      <Dialog
        open={creating}
        onClose={cancelCreating}
        maxWidth="md"
        fullWidth
        slotProps={{
          paper: {
            sx: {
              borderRadius: 2,
              border: "1px solid rgba(120,242,176,0.24)",
              background: "linear-gradient(180deg, rgba(10,14,20,0.98), rgba(6,8,12,0.98))",
              color: ink,
              boxShadow: "0 28px 90px rgba(0,0,0,0.62)",
            },
          },
        }}
      >
        <Box
          component="form"
          onSubmit={(event: FormEvent) => { event.preventDefault(); submit(); }}
          sx={{ m: 0 }}
        >
          <DialogTitle sx={{ px: 3, pt: 2.5, pb: 0.75, color: ink, fontWeight: 800 }}>
            New Canvas
          </DialogTitle>
          <DialogContent sx={{ px: 3, pb: 2.5 }}>
          <Box sx={{
            pt: 1.2,
            display: "grid",
            gap: 1.2,
          }}>
            <Box component="label" htmlFor="arkorbit-create-name" sx={{ fontFamily: monoFont, fontSize: 10.5, letterSpacing: "0.18em", color: warm, textTransform: "uppercase", fontWeight: 700, pr: 1, flexShrink: 0, alignSelf: { xs: "auto", md: "center" } }}>
              Name
            </Box>
            <Box
              component="input"
              id="arkorbit-create-name"
              ref={nameInputRef}
              type="text"
              value={name}
              onChange={(event: ChangeEvent<HTMLInputElement>) => {
                setName(event.target.value);
                setNameError(null);
              }}
              placeholder="Morning command center"
              maxLength={64}
              autoComplete="off"
              autoFocus
              sx={{
                flex: "0 1 300px", minWidth: 0,
                width: "100%",
                boxSizing: "border-box",
                background: "rgba(255,255,255,0.055)", border: "1px solid rgba(255,255,255,0.14)", borderRadius: 1,
                outline: "none", color: ink, caretColor: accent,
                fontFamily: "'Inter', system-ui, sans-serif", fontSize: 14, fontWeight: 600, px: 1.25, py: 1.05,
                cursor: "text", userSelect: "text",
                "&::placeholder": { color: inkDim },
                "&:focus": {
                  borderColor: "rgba(120,242,176,0.7)",
                  boxShadow: "0 0 0 2px rgba(120,242,176,0.14)",
                  background: "rgba(255,255,255,0.075)",
                },
              }}
            />
            {nameError || duplicateName ? (
              <Typography sx={{ color: "#ffb4b4", fontSize: 12, mt: -0.4 }}>
                {nameError || "A canvas with this name already exists."}
              </Typography>
            ) : null}
            <Box component="label" htmlFor="arkorbit-create-icon" sx={{ fontFamily: monoFont, fontSize: 10.5, letterSpacing: "0.18em", color: warm, textTransform: "uppercase", fontWeight: 700, pr: 1, flexShrink: 0, alignSelf: { xs: "auto", md: "center" } }}>
              Glyph
            </Box>
            <Box
              component="input"
              id="arkorbit-create-icon"
              type="text"
              value={icon}
              onChange={(event: ChangeEvent<HTMLInputElement>) => setIcon(event.target.value)}
              placeholder="Optional short mark"
              maxLength={8}
              sx={{
                minWidth: 0,
                width: "100%",
                boxSizing: "border-box",
                background: "rgba(255,255,255,0.055)", border: "1px solid rgba(255,255,255,0.14)", borderRadius: 1,
                outline: "none", color: ink, caretColor: accent,
                fontFamily: "'Inter', system-ui, sans-serif", fontSize: 14, fontWeight: 600, px: 1.25, py: 1.05,
                cursor: "text", userSelect: "text",
                "&::placeholder": { color: inkDim },
                "&:focus": {
                  borderColor: "rgba(120,242,176,0.7)",
                  boxShadow: "0 0 0 2px rgba(120,242,176,0.14)",
                  background: "rgba(255,255,255,0.075)",
                },
              }}
            />
            <Box component="label" htmlFor="arkorbit-create-color" sx={{ fontFamily: monoFont, fontSize: 10.5, letterSpacing: "0.18em", color: warm, textTransform: "uppercase", fontWeight: 700, pr: 1, flexShrink: 0, alignSelf: { xs: "auto", md: "center" } }}>
              Color
            </Box>
            <Box sx={{ display: "flex", alignItems: "center", gap: 1 }}>
              <Box
                component="input"
                id="arkorbit-create-color"
                type="color"
                value={color}
                onChange={(event: ChangeEvent<HTMLInputElement>) => setColor(event.target.value)}
                sx={{
                  width: 54,
                  height: 42,
                  p: 0.4,
                  border: "1px solid rgba(255,255,255,0.14)",
                  borderRadius: 1,
                  background: "rgba(255,255,255,0.055)",
                  cursor: "pointer",
                }}
              />
              <Typography sx={{ color: inkSoft, fontFamily: monoFont, fontSize: 12 }}>
                {color.toUpperCase()}
              </Typography>
            </Box>
            <Box sx={{ display: "flex", gap: 0.8, alignItems: "center", justifyContent: "flex-end", pt: 0.4 }}>
              <Box
                component="button"
                type="button"
                onClick={cancelCreating}
                sx={{
                  px: 1.6, py: 0.9,
                  background: "transparent",
                  border: "1px solid rgba(255,255,255,0.12)",
                  borderRadius: 0.8,
                  color: inkDim,
                  fontFamily: monoFont, fontSize: 10.5, letterSpacing: "0.14em", textTransform: "uppercase", fontWeight: 600,
                  cursor: "pointer",
                  "&:hover:not(:disabled)": { color: ink, borderColor: "rgba(255,255,255,0.24)" },
                  "&:disabled": { opacity: 0.28, cursor: "not-allowed" },
                }}
              >
                Cancel
              </Box>
              <Box
                component="button"
                type="submit"
                disabled={!canSubmit}
                sx={{
                  px: 1.8, py: 0.9,
                  background: "rgba(120,242,176,0.14)",
                  border: "1px solid rgba(120,242,176,0.45)",
                  borderRadius: 0.8,
                  color: accent,
                  fontFamily: monoFont, fontSize: 10.5, letterSpacing: "0.14em", textTransform: "uppercase", fontWeight: 700,
                  cursor: "pointer", flexShrink: 0,
                  "&:hover:not(:disabled)": { background: "rgba(120,242,176,0.24)" },
                  "&:disabled": { opacity: 0.35, cursor: "not-allowed" },
                }}
              >
                Create -&gt;
              </Box>
            </Box>
          </Box>
          </DialogContent>
        </Box>
      </Dialog>

      {latestCanvasOrbits.length > HOME_ORBITS_PER_PAGE ? (
        <Box
          sx={{
            maxWidth: 1280,
            mx: "auto",
            px: { xs: 2, sm: 3, md: 4 },
            mb: 1.6,
            display: "flex",
            alignItems: "center",
            justifyContent: "space-between",
            gap: 1.2,
            flexWrap: "wrap",
          }}
        >
          <Typography
            sx={{
              fontFamily: monoFont,
              fontSize: 10.5,
              letterSpacing: "0.12em",
              color: inkDim,
              textTransform: "uppercase",
              fontVariantNumeric: "tabular-nums",
            }}
          >
            Showing {visibleOrbitStart + 1}-{visibleOrbitEnd} of{" "}
            {latestCanvasOrbits.length}
          </Typography>
          <Stack direction="row" spacing={0.75} sx={{ alignItems: "center" }}>
            <Tooltip title="Previous canvases">
              <span>
                <IconButton
                  size="small"
                  aria-label="Previous canvases"
                  disabled={effectiveOrbitPage === 0}
                  onClick={() => setOrbitPage((page) => Math.max(0, page - 1))}
                  sx={{
                    width: 34,
                    height: 34,
                    border: "1px solid rgba(255,255,255,0.12)",
                    borderRadius: 1,
                    color: inkSoft,
                    "&:hover": {
                      borderColor: "rgba(120,242,176,0.4)",
                      color: accent,
                      background: "rgba(120,242,176,0.08)",
                    },
                    "&.Mui-disabled": { opacity: 0.34, color: inkDim },
                  }}
                >
                  <ChevronLeftRoundedIcon fontSize="small" />
                </IconButton>
              </span>
            </Tooltip>
            <Typography
              sx={{
                minWidth: 72,
                textAlign: "center",
                fontFamily: monoFont,
                fontSize: 10.5,
                color: inkDim,
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {effectiveOrbitPage + 1} / {orbitPageCount}
            </Typography>
            <Tooltip title="Older canvases">
              <span>
                <IconButton
                  size="small"
                  aria-label="Older canvases"
                  disabled={effectiveOrbitPage >= orbitPageCount - 1}
                  onClick={() =>
                    setOrbitPage((page) => Math.min(orbitPageCount - 1, page + 1))
                  }
                  sx={{
                    width: 34,
                    height: 34,
                    border: "1px solid rgba(255,255,255,0.12)",
                    borderRadius: 1,
                    color: inkSoft,
                    "&:hover": {
                      borderColor: "rgba(120,242,176,0.4)",
                      color: accent,
                      background: "rgba(120,242,176,0.08)",
                    },
                    "&.Mui-disabled": { opacity: 0.34, color: inkDim },
                  }}
                >
                  <ChevronRightRoundedIcon fontSize="small" />
                </IconButton>
              </span>
            </Tooltip>
          </Stack>
        </Box>
      ) : null}

      <Box
        sx={{
          maxWidth: 1280,
          mx: "auto",
          px: { xs: 2, sm: 3, md: 4 },
          display: "grid",
          gridTemplateColumns: {
            xs: "repeat(auto-fill, minmax(220px, 1fr))",
            sm: "repeat(auto-fill, minmax(240px, 1fr))",
            md: "repeat(auto-fill, minmax(260px, 1fr))",
          },
          gap: 1.6,
        }}
      >
        {visibleCanvasOrbits.map((orbit, idx) => {
          const absoluteIndex = visibleOrbitStart + idx;
          const tileAccent = accentForOrbit(orbit, absoluteIndex);
          const widgets = widgetCounts[orbit.id];
          const files = fileCounts[orbit.id];
          const initial = orbit.name.trim().charAt(0).toUpperCase() || "·";
          return (
            <Box
              key={orbit.id}
              component="button"
              type="button"
              onClick={() => onSelect(orbit.id)}
              sx={{
                background: panelTint,
                border: "1px solid rgba(255,255,255,0.08)",
                borderRadius: 2,
                overflow: "hidden",
                cursor: "pointer",
                color: "inherit",
                textAlign: "left",
                padding: 0,
                display: "flex",
                flexDirection: "column",
                transition: "all 200ms ease",
                "&:hover": {
                  transform: "translateY(-2px)",
                  borderColor: `${tileAccent}66`,
                  boxShadow: `0 14px 30px -8px rgba(0,0,0,0.6)`,
                },
              }}
            >
              <Box
                sx={{
                  position: "relative",
                  aspectRatio: "16 / 9",
                  background:
                    `radial-gradient(circle at 30% 35%, ${tileAccent}3a, transparent 55%),` +
                    `radial-gradient(circle at 75% 70%, ${tileAccent}1f, transparent 60%),` +
                    "linear-gradient(135deg, rgba(13,17,23,1) 0%, rgba(6,9,12,1) 100%)",
                  borderBottom: "1px solid rgba(255,255,255,0.05)",
                  overflow: "hidden",
                }}
              >
                <Box sx={{ position: "absolute", inset: 0, background: "repeating-linear-gradient(0deg, transparent 0 11px, rgba(255,255,255,0.02) 11px 12px)" }} />
                <Box sx={{
                  position: "absolute", left: 12, top: 12,
                  width: 28, height: 28, borderRadius: 1,
                  border: `1px solid ${tileAccent}55`, background: `${tileAccent}1a`,
                  display: "grid", placeItems: "center",
                  fontFamily: monoFont, fontSize: 13, fontWeight: 700, color: tileAccent,
                }}>
                  {initial}
                </Box>
                {typeof widgets === "number" && widgets > 0 ? (
                  <Box sx={{ position: "absolute", right: 12, bottom: 12, display: "flex", gap: 0.5, alignItems: "center" }}>
                    {Array.from({ length: Math.min(widgets, 4) }).map((_, i) => (
                      <Box key={i} sx={{ width: 10, height: 10, borderRadius: 0.4, background: `${tileAccent}55`, border: `1px solid ${tileAccent}aa` }} />
                    ))}
                    {widgets > 4 ? (
                      <Box sx={{ fontFamily: monoFont, fontSize: 9.5, color: tileAccent, letterSpacing: "0.06em", ml: 0.4 }}>
                        +{widgets - 4}
                      </Box>
                    ) : null}
                  </Box>
                ) : null}
              </Box>

              <Box sx={{ p: { xs: 1.4, md: 1.6 }, flex: 1, display: "flex", flexDirection: "column", gap: 0.6 }}>
                <Box sx={{ display: "flex", alignItems: "baseline", justifyContent: "space-between", gap: 1 }}>
                  <Typography sx={{
                    fontSize: 14.5, fontWeight: 700, letterSpacing: "-0.005em",
                    overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap",
                    flex: 1, minWidth: 0,
                  }}>
                    {orbit.name}
                  </Typography>
                  <Typography sx={{ fontFamily: monoFont, fontSize: 9.5, color: inkDim, letterSpacing: "0.06em", flexShrink: 0 }}>
                    {formatRelative(orbit.updated_at || orbit.created_at)}
                  </Typography>
                </Box>
                {orbit.agent_instructions ? (
                  <Typography sx={{
                    fontSize: 12, color: inkSoft, lineHeight: 1.45,
                    overflow: "hidden", display: "-webkit-box",
                    WebkitLineClamp: 2, WebkitBoxOrient: "vertical",
                  }}>
                    {orbit.agent_instructions}
                  </Typography>
                ) : null}
                <Box sx={{
                  display: "flex", gap: 1.4,
                  pt: 0.8, mt: "auto",
                  borderTop: `1px dashed ${ruleSoft}`,
                  fontFamily: monoFont, fontSize: 10, color: inkDim, letterSpacing: "0.06em",
                }}>
                  <Box>
                    <Box component="strong" sx={{ color: ink, fontWeight: 700, mr: 0.4 }}>
                      {typeof widgets === "number" ? widgets : "—"}
                    </Box>
                    widget{widgets === 1 ? "" : "s"}
                  </Box>
                  <Box>
                    <Box component="strong" sx={{ color: ink, fontWeight: 700, mr: 0.4 }}>
                      {typeof files === "number" ? files : "—"}
                    </Box>
                    file{files === 1 ? "" : "s"}
                  </Box>
                </Box>
              </Box>
            </Box>
          );
        })}

        <Box
          component="button"
          type="button"
          onClick={startCreating}
          sx={{
            background: "repeating-linear-gradient(45deg, transparent 0 6px, rgba(120,242,176,0.05) 6px 7px), rgba(13,17,23,0.4)",
            border: "1px dashed rgba(120,242,176,0.32)",
            borderRadius: 2,
            cursor: "pointer", color: "inherit",
            minHeight: 200,
            display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
            gap: 1.2, padding: 3,
            transition: "all 200ms ease",
            "&:hover": {
              background: "repeating-linear-gradient(45deg, transparent 0 6px, rgba(120,242,176,0.10) 6px 7px), rgba(120,242,176,0.04)",
              borderColor: "rgba(120,242,176,0.6)",
            },
          }}
        >
          <Box sx={{
            width: 44, height: 44, borderRadius: "50%",
            border: "1px solid rgba(120,242,176,0.55)",
            display: "grid", placeItems: "center",
            color: accent, fontSize: 24, fontWeight: 300,
          }}>+</Box>
          <Typography sx={{ fontFamily: monoFont, fontSize: 11, letterSpacing: "0.18em", color: accent, textTransform: "uppercase", fontWeight: 700 }}>
            New Canvas
          </Typography>
          <Typography sx={{ fontSize: 12, color: inkSoft, textAlign: "center", maxWidth: 200, lineHeight: 1.45 }}>
            Name it, pick a color, then open the canvas.
          </Typography>
        </Box>
      </Box>
    </Box>
  );
}

export function ArkOrbitPage() {
  const initialRouteState = useMemo(readOrbitRouteState, []);
  const [orbits, setOrbits] = useState<Orbit[]>([]);
  const [activeOrbitId, setActiveOrbitId] = useState<OrbitId | null>(null);
  const [settingsOrbitId, setSettingsOrbitId] = useState<OrbitId | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [runtimeNotices, setRuntimeNotices] = useState<string[]>([]);
  const [bootstrapping, setBootstrapping] = useState(true);
  const [frameVersion, setFrameVersion] = useState(0);
  const [orbitReloadSignal, setOrbitReloadSignal] = useState(0);
  const [chatOpen, setChatOpen] = useState(initialRouteState.chatOpen);
  const [chatAnchor, setChatAnchor] = useState<ChatAnchor | null>(null);
  const runtimeNoticeUseCountsRef = useRef<Record<string, number>>({});
  const canvasRef = useRef<HTMLDivElement | null>(null);
  const chatDragRef = useRef<DragState | null>(null);
  const chatMovedRef = useRef(false);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const result = await arkorbitApi.listOrbits();
        if (cancelled) return;
        const requestedOrbit = initialRouteState.orbitId
          ? result.orbits.find((orbit) => orbit.id === initialRouteState.orbitId)
          : null;
        setOrbits(result.orbits);
        setActiveOrbitId(requestedOrbit?.id ?? result.orbits[0]?.id ?? null);
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
  }, [initialRouteState.orbitId]);

  const activeOrbit = useMemo(
    () => orbits.find((orbit) => orbit.id === activeOrbitId) ?? null,
    [activeOrbitId, orbits],
  );
  const showHomeDashboard = Boolean(activeOrbit?.is_default);

  useEffect(() => {
    if (bootstrapping) return;
    writeOrbitRouteState(activeOrbit, chatOpen && !showHomeDashboard);
  }, [activeOrbit, bootstrapping, chatOpen, showHomeDashboard]);

  const handleSelectOrbit = useCallback((id: OrbitId) => {
    setActiveOrbitId(id);
    setFrameVersion((prev) => prev + 1);
    setRuntimeNotices([]);
    runtimeNoticeUseCountsRef.current = {};
  }, []);

  const handleOrbitCreated = useCallback((orbit: Orbit) => {
    setOrbits((prev) => [...prev, orbit]);
    setActiveOrbitId(orbit.id);
    setFrameVersion((prev) => prev + 1);
    setRuntimeNotices([]);
    runtimeNoticeUseCountsRef.current = {};
    setChatOpen(true);
  }, []);

  const handleRuntimeNotice = useCallback((message: string) => {
    const trimmed = message.trim();
    if (!trimmed) return;
    setLoadError(trimmed);
    const budgetKey = normalizeRuntimeNoticeForBudget(trimmed);
    const currentUses = runtimeNoticeUseCountsRef.current[budgetKey] ?? 0;
    if (currentUses >= MAX_RUNTIME_NOTICE_USES) return;
    setRuntimeNotices((prev) => {
      const next = [trimmed, ...prev.filter((item) => item !== trimmed)];
      return next.slice(0, 6);
    });
  }, []);

  const runtimeNoticesForNextTurn = useMemo(
    () =>
      runtimeNotices.filter((notice) => {
        const budgetKey = normalizeRuntimeNoticeForBudget(notice);
        return (runtimeNoticeUseCountsRef.current[budgetKey] ?? 0) < MAX_RUNTIME_NOTICE_USES;
      }),
    [runtimeNotices],
  );

  const markRuntimeNoticesUsed = useCallback((notices: string[]) => {
    if (notices.length === 0) return;
    for (const notice of notices) {
      const budgetKey = normalizeRuntimeNoticeForBudget(notice);
      if (!budgetKey) continue;
      runtimeNoticeUseCountsRef.current[budgetKey] =
        (runtimeNoticeUseCountsRef.current[budgetKey] ?? 0) + 1;
    }
    setRuntimeNotices((prev) =>
      prev.filter((notice) => {
        const budgetKey = normalizeRuntimeNoticeForBudget(notice);
        return (runtimeNoticeUseCountsRef.current[budgetKey] ?? 0) < MAX_RUNTIME_NOTICE_USES;
      }),
    );
  }, []);

  const handleCanvasCreateSubmit = useCallback(
    async (payload: CreateOrbitPayload) => {
      try {
      } catch {
        // sessionStorage unavailable — orbit will still be created, prompt just won't prefill
      }
      try {
        const created = await arkorbitApi.createOrbit(payload);
        if (created) {
          handleOrbitCreated(created);
        } else {
          setLoadError("Could not create canvas. Try again.");
        }
      } catch (error) {
        setLoadError(error instanceof Error ? error.message : String(error));
      }
    },
    [handleOrbitCreated],
  );

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
              onSelect={handleSelectOrbit}
              onCreateSubmit={handleCanvasCreateSubmit}
            />
          ) : activeOrbitId ? (
            <OrbitFrame
              key={`${activeOrbitId}:${frameVersion}`}
              orbitId={activeOrbitId}
              externalReloadToken={orbitReloadSignal}
              onRuntimeNotice={handleRuntimeNotice}
            />
          ) : (
            <Box className="orbit-home-canvas">
              <Typography variant="body2" sx={{ color: "text.secondary" }}>
                {bootstrapping ? "Loading orbit..." : "No orbit available."}
              </Typography>
            </Box>
          )}
        </Box>
        {activeOrbitId && !showHomeDashboard && !chatOpen ? (
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
        {activeOrbitId && !showHomeDashboard && chatOpen ? (
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
              runtimeNotices={runtimeNoticesForNextTurn}
              onRuntimeNoticesSubmitted={markRuntimeNoticesUsed}
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
