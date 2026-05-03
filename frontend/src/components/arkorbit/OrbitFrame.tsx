import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type PointerEvent,
} from "react";
import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  IconButton,
  Stack,
  Tooltip,
  Typography,
} from "@mui/material";
import CloseRoundedIcon from "@mui/icons-material/CloseRounded";
import CodeRoundedIcon from "@mui/icons-material/CodeRounded";
import FolderOpenRoundedIcon from "@mui/icons-material/FolderOpenRounded";
import InsertDriveFileRoundedIcon from "@mui/icons-material/InsertDriveFileRounded";
import { arkorbitApi } from "./api";
import type { OrbitFileEntry, OrbitId } from "./types";

type Props = {
  orbitId: OrbitId;
  externalReloadToken?: number;
  onRuntimeNotice?: (message: string) => void;
};

type OrbitWidgetRegistryEntry = {
  id?: string;
  module?: string;
  title?: string;
  left?: number;
  top?: number;
  width?: number;
  height?: number;
  [key: string]: unknown;
};

type OrbitWidgetModule = {
  render?: (
    el: HTMLElement,
    ctx: OrbitWidgetContext,
  ) => void | (() => void) | Promise<void | (() => void)>;
};

type OrbitWidgetContext = {
  orbitId: OrbitId;
  widget: OrbitWidgetRegistryEntry;
  moduleName: string;
  resolveText: (path: string) => Promise<string>;
  importMod: (path: string) => Promise<unknown>;
  fetchPublic: (input: OrbitFetchInput, init?: RequestInit) => Promise<Response>;
  fetchText: (input: OrbitFetchInput, init?: RequestInit) => Promise<string>;
  fetchJson: <T = unknown>(input: OrbitFetchInput, init?: RequestInit) => Promise<T>;
};

type DragState = {
  pointerId: number;
  startX: number;
  startY: number;
  originX: number;
  originY: number;
};

type OrbitFetchInput = string | URL | Request;
type WidgetLayout = { x: number; y: number; width: number; height: number };
type ViewportRect = { left: number; top: number; width: number; height: number };
type WidgetIdentity = { id: string; moduleName: string };
type WidgetRemovalTarget = WidgetIdentity & { title: string };

const WIDGET_POSITION_STORAGE_VERSION = "v2";
const INTERACTIVE_SELECTOR =
  'button,a,input,textarea,select,[contenteditable="true"],[data-no-drag]';
const DEFAULT_WIDGET_WIDTH = 340;
const DEFAULT_WIDGET_HEIGHT = 180;
const WIDGET_PLACEMENT_GAP = 24;
const WIDGET_PLACEMENT_STEP = 32;
const WIDGET_VIEWPORT_MARGIN = 32;
const WIDGET_TOP_VIEWPORT_MARGIN = 88;

function normalizeOrbitModulePath(path: string): string | null {
  const trimmed = path.trim();
  if (
    !trimmed ||
    trimmed.length > 512 ||
    trimmed.includes("\0") ||
    trimmed.includes("\\") ||
    trimmed.includes("?") ||
    trimmed.includes("#") ||
    /^[a-z][a-z0-9+.-]*:/i.test(trimmed) ||
    trimmed.startsWith("/")
  ) {
    return null;
  }
  const parts = trimmed.split("/").filter(Boolean);
  if (parts.length === 0 || parts.length > 16) return null;
  if (parts.some((part) => part === "." || part === "..")) return null;
  return parts.join("/");
}

function normalizeWidgetModuleName(widget: OrbitWidgetRegistryEntry): string | null {
  const raw = String(widget.module || widget.id || "").trim();
  if (!raw) return null;
  const withoutModPrefix = raw.replace(/^mod\//, "");
  const moduleName = withoutModPrefix.replace(/\/index\.js$/, "");
  if (!moduleName || moduleName.includes("/")) return null;
  return normalizeOrbitModulePath(moduleName);
}

async function fetchOrbitText(orbitId: OrbitId, path: string): Promise<string> {
  const normalizedPath = normalizeOrbitModulePath(path);
  if (!normalizedPath) throw new Error("Rejected unsafe orbit module path.");
  const response = await fetch(arkorbitApi.moduleUrl(orbitId, normalizedPath), {
    credentials: "include",
    cache: "no-store",
  });
  const content = await response.text();
  if (!response.ok) {
    throw new Error(content || `Orbit file request failed (${response.status}).`);
  }
  return content;
}

async function importOrbitModule(
  orbitId: OrbitId,
  path: string,
): Promise<unknown> {
  const source = await fetchOrbitText(orbitId, path);
  const moduleSource = `${buildOrbitFetchShim(orbitId)}\n${source}\n//# sourceURL=arkorbit://${orbitId}/${path}`;
  const blobUrl = URL.createObjectURL(
    new Blob([moduleSource], { type: "text/javascript" }),
  );
  try {
    return await import(/* @vite-ignore */ blobUrl);
  } finally {
    URL.revokeObjectURL(blobUrl);
  }
}

function parseRegistry(raw: string): OrbitWidgetRegistryEntry[] {
  const parsed = JSON.parse(raw) as unknown;
  const list =
    Array.isArray(parsed)
      ? parsed
      : parsed && typeof parsed === "object"
        ? (parsed as { widgets?: unknown }).widgets
        : [];
  if (!Array.isArray(list)) return [];
  return list.filter(
    (entry): entry is OrbitWidgetRegistryEntry =>
      !!entry && typeof entry === "object",
  );
}

async function fetchWidgetRegistry(
  orbitId: OrbitId,
): Promise<OrbitWidgetRegistryEntry[]> {
  try {
    return parseRegistry(await fetchOrbitText(orbitId, "data/widgets.json"));
  } catch (error) {
    const text = error instanceof Error ? error.message : String(error);
    if (text.includes("404") || text.includes("not found")) return [];
    throw error;
  }
}

function orbitChangedPath(event: MessageEvent): string | null {
  if (typeof event.data !== "string") return null;
  try {
    const payload = JSON.parse(event.data) as { path?: unknown };
    return typeof payload.path === "string" ? payload.path : null;
  } catch {
    return null;
  }
}

function shouldReloadForPath(path: string | null): boolean {
  if (!path) return false;
  return (
    path === "index.html" ||
    path === "data/widgets.json" ||
    path.startsWith("mod/") ||
    path.startsWith("assets/")
  );
}

function storageKey(orbitId: OrbitId): string {
  return `arkorbit:${orbitId}:widget-positions:${WIDGET_POSITION_STORAGE_VERSION}`;
}

function readStoredPositions(orbitId: OrbitId): Record<string, { x: number; y: number }> {
  try {
    const parsed = JSON.parse(localStorage.getItem(storageKey(orbitId)) || "{}");
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function removeStoredPosition(orbitId: OrbitId, id: string) {
  try {
    const positions = readStoredPositions(orbitId);
    delete positions[id];
    localStorage.setItem(storageKey(orbitId), JSON.stringify(positions));
  } catch {
    // Browser storage can be disabled. Deleting the widget should still work.
  }
}

function numberValue(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function widgetId(widget: OrbitWidgetRegistryEntry, moduleName: string): string {
  const raw = String(widget.id || moduleName).trim();
  return raw || moduleName;
}

function widgetIdentity(
  widget: OrbitWidgetRegistryEntry,
  index: number,
): WidgetIdentity | null {
  const moduleName = normalizeWidgetModuleName(widget);
  if (!moduleName) return null;
  return {
    moduleName,
    id: widgetId(widget, moduleName) || `widget-${index}`,
  };
}

function widgetSize(widget: OrbitWidgetRegistryEntry): Pick<WidgetLayout, "width" | "height"> {
  return {
    width: numberValue(widget.width, DEFAULT_WIDGET_WIDTH),
    height: numberValue(widget.height, DEFAULT_WIDGET_HEIGHT),
  };
}

function widgetRemovalTarget(
  widgets: OrbitWidgetRegistryEntry[],
  id: string | null,
): WidgetRemovalTarget | null {
  if (!id) return null;
  for (let index = 0; index < widgets.length; index += 1) {
    const widget = widgets[index];
    const identity = widgetIdentity(widget, index);
    if (!identity || identity.id !== id) continue;
    return {
      ...identity,
      title: String(widget.title || identity.moduleName),
    };
  }
  return null;
}

function currentViewport(node: HTMLElement | null): ViewportRect {
  return {
    left: node?.scrollLeft ?? 0,
    top: node?.scrollTop ?? 0,
    width: node?.clientWidth ?? 1200,
    height: node?.clientHeight ?? 800,
  };
}

function rectsOverlap(a: WidgetLayout, b: WidgetLayout, gap = WIDGET_PLACEMENT_GAP): boolean {
  return !(
    a.x + a.width + gap <= b.x ||
    b.x + b.width + gap <= a.x ||
    a.y + a.height + gap <= b.y ||
    b.y + b.height + gap <= a.y
  );
}

function collidesWithOccupied(candidate: WidgetLayout, occupied: WidgetLayout[]): boolean {
  return occupied.some((rect) => rectsOverlap(candidate, rect));
}

function measureMountedWidgetRects(root: HTMLElement | null): Record<string, WidgetLayout> {
  if (!root) return {};
  const out: Record<string, WidgetLayout> = {};
  root.querySelectorAll<HTMLElement>("[data-orbit-widget-id]").forEach((node) => {
    const id = node.dataset.orbitWidgetId;
    if (!id) return;
    const x = Number.parseFloat(node.style.left) || node.offsetLeft || 0;
    const y = Number.parseFloat(node.style.top) || node.offsetTop || 0;
    out[id] = {
      x,
      y,
      width: Math.max(node.offsetWidth, node.scrollWidth, 1),
      height: Math.max(node.offsetHeight, node.scrollHeight, 1),
    };
  });
  return out;
}

function findClosestEmptyLayout(
  occupied: WidgetLayout[],
  viewport: ViewportRect,
  size: Pick<WidgetLayout, "width" | "height">,
): WidgetLayout {
  const safeWidth = Math.max(180, size.width);
  const safeHeight = Math.max(120, size.height);
  const anchorX = Math.max(0, viewport.left + WIDGET_VIEWPORT_MARGIN);
  const anchorY = Math.max(
    0,
    viewport.top + (viewport.top < WIDGET_TOP_VIEWPORT_MARGIN ? WIDGET_TOP_VIEWPORT_MARGIN : WIDGET_VIEWPORT_MARGIN),
  );
  const visibleRight = Math.max(anchorX, viewport.left + viewport.width - safeWidth - WIDGET_VIEWPORT_MARGIN);
  const visibleBottom = Math.max(anchorY, viewport.top + viewport.height - safeHeight - WIDGET_VIEWPORT_MARGIN);

  let best: WidgetLayout | null = null;
  let bestScore = Number.POSITIVE_INFINITY;
  const consider = (x: number, y: number) => {
    const candidate = { x: Math.max(0, x), y: Math.max(0, y), width: safeWidth, height: safeHeight };
    if (collidesWithOccupied(candidate, occupied)) return;
    const score =
      Math.abs(candidate.x - anchorX) +
      Math.abs(candidate.y - anchorY) +
      candidate.y * 0.01 +
      candidate.x * 0.001;
    if (score < bestScore) {
      bestScore = score;
      best = candidate;
    }
  };

  for (let y = anchorY; y <= visibleBottom; y += WIDGET_PLACEMENT_STEP) {
    for (let x = anchorX; x <= visibleRight; x += WIDGET_PLACEMENT_STEP) {
      consider(x, y);
    }
  }

  if (!best) {
    const expandedRight = viewport.left + Math.max(viewport.width * 2, safeWidth + 800);
    const expandedBottom = viewport.top + Math.max(viewport.height * 2, safeHeight + 800);
    for (let y = anchorY; y <= expandedBottom; y += WIDGET_PLACEMENT_STEP) {
      for (let x = anchorX; x <= expandedRight; x += WIDGET_PLACEMENT_STEP) {
        consider(x, y);
      }
    }
  }

  return best ?? {
    x: anchorX,
    y: occupied.reduce((bottom, rect) => Math.max(bottom, rect.y + rect.height + WIDGET_PLACEMENT_GAP), anchorY),
    width: safeWidth,
    height: safeHeight,
  };
}

function resolveWidgetLayouts(
  orbitId: OrbitId,
  widgets: OrbitWidgetRegistryEntry[],
  viewport: ViewportRect,
  mountedRects: Record<string, WidgetLayout>,
): Record<string, WidgetLayout> {
  const stored = readStoredPositions(orbitId);
  const occupied: WidgetLayout[] = [];
  const layouts: Record<string, WidgetLayout> = {};

  widgets.forEach((widget, index) => {
    const identity = widgetIdentity(widget, index);
    if (!identity) return;
    const size = widgetSize(widget);
    const savedLeft = numberValue(widget.left, Number.NaN);
    const savedTop = numberValue(widget.top, Number.NaN);
    const savedPosition =
      Number.isFinite(savedLeft) && Number.isFinite(savedTop)
        ? { x: savedLeft, y: savedTop }
        : null;
    const storedPosition = savedPosition ?? stored[identity.id];
    const mounted = mountedRects[identity.id];
    let layout: WidgetLayout;

    if (storedPosition) {
      layout = {
        x: storedPosition.x,
        y: storedPosition.y,
        width: mounted?.width ?? size.width,
        height: mounted?.height ?? size.height,
      };
    } else {
      layout = findClosestEmptyLayout(occupied, viewport, size);
    }

    layouts[identity.id] = layout;
    occupied.push(layout);
  });

  return layouts;
}

function requestUrl(input: OrbitFetchInput): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  if (input instanceof Request) return input.url;
  return "";
}

function requestMethod(input: OrbitFetchInput, init?: RequestInit): string {
  return String(init?.method || (input instanceof Request ? input.method : "GET") || "GET").toUpperCase();
}

function shouldProxyFetch(input: OrbitFetchInput, init?: RequestInit): string | null {
  const raw = requestUrl(input).trim();
  if (!raw) return null;
  let parsed: URL;
  try {
    parsed = new URL(raw, window.location.href);
  } catch {
    return null;
  }
  if (!["http:", "https:"].includes(parsed.protocol)) return null;
  if (parsed.origin === window.location.origin) return null;
  const method = requestMethod(input, init);
  if (method !== "GET" && method !== "HEAD") return null;
  return parsed.toString();
}

function proxyFetchHeaders(input: OrbitFetchInput, init?: RequestInit): Headers {
  const source = new Headers(input instanceof Request ? input.headers : undefined);
  if (init?.headers) {
    new Headers(init.headers).forEach((value, key) => source.set(key, value));
  }
  const forwarded = new Headers();
  ["accept", "accept-language", "if-none-match", "if-modified-since", "range"].forEach((key) => {
    const value = source.get(key);
    if (value) forwarded.set(key, value);
  });
  return forwarded;
}

function createOrbitPublicFetch(orbitId: OrbitId) {
  return (input: OrbitFetchInput, init?: RequestInit): Promise<Response> => {
    const publicUrl = shouldProxyFetch(input, init);
    if (!publicUrl) {
      return fetch(input, init);
    }
    return fetch(arkorbitApi.orbitPublicFetchUrl(orbitId, publicUrl), {
      method: requestMethod(input, init),
      credentials: "include",
      cache: "no-store",
      signal: init?.signal,
      headers: proxyFetchHeaders(input, init),
    });
  };
}

function buildOrbitFetchShim(orbitId: OrbitId): string {
  const proxyPrefix = arkorbitApi.orbitPublicFetchUrlPrefix(orbitId);
  return `
const __arkorbitNativeFetch = globalThis.fetch.bind(globalThis);
const __arkorbitProxyPrefix = ${JSON.stringify(proxyPrefix)};
const __arkorbitProxyableFetchUrl = (input, init) => {
  const raw = typeof input === "string"
    ? input
    : input instanceof URL
      ? input.toString()
      : input && typeof input.url === "string"
        ? input.url
        : "";
  if (!/^https?:\\/\\//i.test(raw)) return null;
  let parsed;
  try { parsed = new URL(raw); } catch { return null; }
  if (parsed.origin === globalThis.location.origin) return null;
  const method = String((init && init.method) || (input && input.method) || "GET").toUpperCase();
  if (method !== "GET" && method !== "HEAD") return null;
  return parsed.toString();
};
const fetch = (input, init = undefined) => {
  const publicUrl = __arkorbitProxyableFetchUrl(input, init);
  if (!publicUrl) return __arkorbitNativeFetch(input, init);
  const proxyInit = Object.assign({}, init || {}, {
    method: String((init && init.method) || (input && input.method) || "GET").toUpperCase(),
    credentials: "include",
    cache: "no-store"
  });
  delete proxyInit.body;
  return __arkorbitNativeFetch(__arkorbitProxyPrefix + encodeURIComponent(publicUrl), proxyInit);
};
`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function fileKind(path: string): string {
  if (path.endsWith(".js") || path.endsWith(".mjs")) return "JS";
  if (path.endsWith(".json") || path.endsWith(".jsonl")) return "JSON";
  if (path.endsWith(".html")) return "HTML";
  if (path.endsWith(".css")) return "CSS";
  if (path.endsWith(".md")) return "MD";
  return "FILE";
}

async function readOrbitFile(orbitId: OrbitId, path: string): Promise<string> {
  const normalizedPath = normalizeOrbitModulePath(path);
  if (!normalizedPath) throw new Error("Rejected unsafe orbit file path.");
  const response = await fetch(arkorbitApi.orbitFileUrl(orbitId, normalizedPath), {
    credentials: "include",
    cache: "no-store",
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(text || `Orbit file request failed (${response.status}).`);
  }
  return text;
}

type OrbitFilesPanelProps = {
  orbitId: OrbitId;
  reloadToken: number;
  onRuntimeNotice?: (message: string) => void;
};

function OrbitFilesPanel({
  orbitId,
  reloadToken,
  onRuntimeNotice,
}: OrbitFilesPanelProps) {
  const [open, setOpen] = useState(false);
  const [files, setFiles] = useState<OrbitFileEntry[]>([]);
  const [activePath, setActivePath] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [loadingFiles, setLoadingFiles] = useState(true);
  const [loadingContent, setLoadingContent] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoadingFiles(true);
    void arkorbitApi
      .listFiles(orbitId)
      .then((next) => {
        if (cancelled) return;
        setError(null);
        setFiles(next);
        setActivePath((current) =>
          current && next.some((file) => file.path === current)
            ? current
            : next[0]?.path ?? null,
        );
      })
      .catch((err) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        setFiles([]);
        setError(message);
        onRuntimeNotice?.(message);
      })
      .finally(() => {
        if (!cancelled) setLoadingFiles(false);
      });
    return () => {
      cancelled = true;
    };
  }, [orbitId, onRuntimeNotice, reloadToken]);

  useEffect(() => {
    if (!open) return undefined;
    if (!activePath) {
      setContent("");
      setError(null);
      return undefined;
    }
    let cancelled = false;
    setLoadingContent(true);
    setError(null);
    void readOrbitFile(orbitId, activePath)
      .then((text) => {
        if (!cancelled) setContent(text);
      })
      .catch((err) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        setContent("");
        setError(message);
        onRuntimeNotice?.(message);
      })
      .finally(() => {
        if (!cancelled) setLoadingContent(false);
      });
    return () => {
      cancelled = true;
    };
  }, [activePath, open, orbitId, onRuntimeNotice, reloadToken]);

  if (!open) {
    return (
      <button
        type="button"
        className="orbit-files-collapsed"
        onClick={() => setOpen(true)}
        aria-label="Open Orbit files"
      >
        <FolderOpenRoundedIcon fontSize="small" />
        <span>Files</span>
        <strong>{loadingFiles ? "..." : files.length}</strong>
      </button>
    );
  }

  return (
    <Box className="orbit-files-panel" role="region" aria-label="Orbit files">
      <Box className="orbit-files-header">
        <Box className="orbit-files-title">
          <CodeRoundedIcon fontSize="small" />
          <span>Files</span>
          <strong>{files.length}</strong>
        </Box>
        <Tooltip title="Collapse files">
          <IconButton
            size="small"
            className="orbit-files-close"
            onClick={() => setOpen(false)}
            aria-label="Collapse Orbit files"
          >
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Box>
      <Box className="orbit-files-content">
        <Box className="orbit-files-list" role="listbox" aria-label="Orbit file list">
          {files.map((file) => (
            <button
              key={file.path}
              type="button"
              role="option"
              className={`orbit-file-row${file.path === activePath ? " is-active" : ""}`}
              onClick={() => setActivePath(file.path)}
              aria-selected={file.path === activePath}
            >
              <InsertDriveFileRoundedIcon fontSize="small" />
              <span className="orbit-file-name">{file.path}</span>
              <span className="orbit-file-meta">
                {fileKind(file.path)} - {formatBytes(file.bytes)}
              </span>
            </button>
          ))}
          {!loadingFiles && files.length === 0 ? (
            <Box className="orbit-files-empty">No files</Box>
          ) : null}
        </Box>
        <Box className="orbit-file-viewer">
          <Box className="orbit-file-viewer-header">
            <span>{activePath ?? "No file"}</span>
            {activePath ? <strong>{fileKind(activePath)}</strong> : null}
          </Box>
          <pre className="orbit-file-code" tabIndex={0}>
            <code>
              {error
                ? error
                : loadingContent
                  ? "Loading..."
                  : activePath
                    ? content
                    : ""}
            </code>
          </pre>
        </Box>
      </Box>
    </Box>
  );
}

type OrbitWidgetSlotProps = {
  orbitId: OrbitId;
  widget: OrbitWidgetRegistryEntry;
  index: number;
  layout: WidgetLayout;
  reloadToken: number;
  onRemove: (id: string) => void;
  onMove: (id: string, x: number, y: number) => void;
  onRuntimeNotice?: (message: string) => void;
};

function OrbitWidgetSlot({
  orbitId,
  widget,
  index,
  layout,
  reloadToken,
  onRemove,
  onMove,
  onRuntimeNotice,
}: OrbitWidgetSlotProps) {
  const hostRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<DragState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [rendering, setRendering] = useState(false);
  const identity = useMemo(() => widgetIdentity(widget, index), [index, widget]);
  const moduleName = identity?.moduleName ?? null;
  const id = identity?.id ?? `widget-${index}`;

  const style = useMemo(() => {
    return {
      left: `${layout.x}px`,
      top: `${layout.y}px`,
      width: `${layout.width}px`,
      minHeight: `${layout.height}px`,
    } satisfies CSSProperties;
  }, [layout.height, layout.width, layout.x, layout.y]);

  useEffect(() => {
    let disposed = false;
    let cleanup: void | (() => void);
    const host = hostRef.current;
    if (!host || !moduleName) return undefined;

    setError(null);
    setRendering(true);
    host.innerHTML = "";
    const fetchPublic = createOrbitPublicFetch(orbitId);
    const ctx: OrbitWidgetContext = {
      orbitId,
      widget,
      moduleName,
      resolveText: (path: string) => fetchOrbitText(orbitId, path),
      importMod: (path: string) => importOrbitModule(orbitId, path),
      fetchPublic,
      fetchText: async (input, init) => {
        const response = await fetchPublic(input, init);
        if (!response.ok) throw new Error(`Fetch failed (${response.status})`);
        return response.text();
      },
      fetchJson: async (input, init) => {
        const response = await fetchPublic(input, init);
        if (!response.ok) throw new Error(`Fetch failed (${response.status})`);
        return response.json();
      },
    };

    void importOrbitModule(orbitId, `${moduleName}/index.js`)
      .then(async (module) => {
        if (disposed) return;
        const render = (module as OrbitWidgetModule).render;
        if (typeof render !== "function") {
          throw new Error("Widget module must export render(el, ctx).");
        }
        cleanup = await render(host, ctx);
        setRendering(false);
      })
      .catch((err) => {
        if (disposed) return;
        const message = err instanceof Error ? err.message : String(err);
        setError(message);
        setRendering(false);
        onRuntimeNotice?.(message);
      });

    return () => {
      disposed = true;
      if (typeof cleanup === "function") {
        try {
          cleanup();
        } catch {
          // Widget cleanup should not break route teardown.
        }
      }
      host.innerHTML = "";
    };
  }, [moduleName, onRuntimeNotice, orbitId, reloadToken, widget]);

  const handlePointerDown = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      if (event.button !== 0) return;
      const target = event.target;
      const handle =
        target instanceof Element ? target.closest("[data-orbit-drag-handle]") : null;
      if (!handle && target instanceof Element && target.closest(INTERACTIVE_SELECTOR)) {
        return;
      }
      const rect = event.currentTarget.getBoundingClientRect();
      const parentRect =
        event.currentTarget.offsetParent?.getBoundingClientRect() ??
        new DOMRect(0, 0, 0, 0);
      dragRef.current = {
        pointerId: event.pointerId,
        startX: event.clientX,
        startY: event.clientY,
        originX: Number.parseFloat(event.currentTarget.style.left) || rect.left - parentRect.left,
        originY: Number.parseFloat(event.currentTarget.style.top) || rect.top - parentRect.top,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
      event.preventDefault();
    },
    [],
  );

  const handlePointerMove = useCallback((event: PointerEvent<HTMLDivElement>) => {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    const x = Math.max(0, drag.originX + event.clientX - drag.startX);
    const y = Math.max(0, drag.originY + event.clientY - drag.startY);
    event.currentTarget.style.left = `${x}px`;
    event.currentTarget.style.top = `${y}px`;
  }, []);

  const finishDrag = useCallback(
    (event: PointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      dragRef.current = null;
      const x = Number.parseFloat(event.currentTarget.style.left) || 0;
      const y = Number.parseFloat(event.currentTarget.style.top) || 0;
      onMove(id, x, y);
      try {
        event.currentTarget.releasePointerCapture(event.pointerId);
      } catch {
        // Pointer capture may already be released by the browser.
      }
    },
    [id, onMove],
  );

  if (!moduleName) {
    return null;
  }

  return (
    <Box
      className="orbit-widget-shell"
      data-orbit-widget="true"
      data-orbit-widget-id={id}
      style={style}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={finishDrag}
      onPointerCancel={finishDrag}
    >
      <Box className="orbit-widget-toolbar" data-no-drag="true">
        <Tooltip title="Remove widget">
          <IconButton
            size="small"
            className="orbit-widget-action"
            data-no-drag="true"
            aria-label={`Remove ${widget.title ?? moduleName} widget`}
            onPointerDown={(event) => event.stopPropagation()}
            onClick={(event) => {
              event.stopPropagation();
              onRemove(id);
            }}
          >
            <CloseRoundedIcon fontSize="small" />
          </IconButton>
        </Tooltip>
      </Box>
      {rendering && !error ? (
        <Box className="orbit-widget-loading">Loading {widget.title ?? moduleName ?? "widget"}...</Box>
      ) : null}
      {error ? <Box className="orbit-widget-error">{error}</Box> : null}
      <div ref={hostRef} className="orbit-widget-body" />
    </Box>
  );
}

export function OrbitFrame({
  orbitId,
  externalReloadToken = 0,
  onRuntimeNotice,
}: Props) {
  const frameShellRef = useRef<HTMLDivElement | null>(null);
  const reloadTimerRef = useRef<number | null>(null);
  const [widgets, setWidgets] = useState<OrbitWidgetRegistryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [reloadToken, setReloadToken] = useState(0);
  const [filesReloadToken, setFilesReloadToken] = useState(0);
  const [pendingRemoveWidgetId, setPendingRemoveWidgetId] = useState<string | null>(null);
  const [removingWidget, setRemovingWidget] = useState(false);

  const reload = useCallback(() => {
    if (reloadTimerRef.current !== null) {
      window.clearTimeout(reloadTimerRef.current);
    }
    reloadTimerRef.current = window.setTimeout(() => {
      reloadTimerRef.current = null;
      setReloadToken((prev) => prev + 1);
    }, 120);
  }, []);

  const removeWidget = useCallback(
    async (id: string) => {
      const previous = widgets;
      removeStoredPosition(orbitId, id);
      setWidgets((current) =>
        current.filter((widget, index) => widgetIdentity(widget, index)?.id !== id),
      );
      setFilesReloadToken((prev) => prev + 1);
      try {
        await arkorbitApi.deleteWidget(orbitId, id);
        reload();
        return true;
      } catch (err) {
        setWidgets(previous);
        const message = err instanceof Error ? err.message : String(err);
        onRuntimeNotice?.(`Could not remove widget: ${message}`);
        return false;
      }
    },
    [orbitId, onRuntimeNotice, reload, widgets],
  );

  const moveWidget = useCallback(
    (id: string, x: number, y: number) => {
      setWidgets((current) =>
        current.map((widget, index) => {
          const identity = widgetIdentity(widget, index);
          return identity?.id === id ? { ...widget, left: x, top: y } : widget;
        }),
      );
      void arkorbitApi.updateWidgetLayout(orbitId, id, { left: x, top: y }).catch((err) => {
        const message = err instanceof Error ? err.message : String(err);
        onRuntimeNotice?.(`Could not save widget position: ${message}`);
      });
    },
    [orbitId, onRuntimeNotice],
  );

  const pendingRemoveWidget = useMemo(
    () => widgetRemovalTarget(widgets, pendingRemoveWidgetId),
    [pendingRemoveWidgetId, widgets],
  );

  const confirmRemoveWidget = useCallback(async () => {
    if (!pendingRemoveWidgetId) return;
    setRemovingWidget(true);
    try {
      const removed = await removeWidget(pendingRemoveWidgetId);
      if (removed) setPendingRemoveWidgetId(null);
    } finally {
      setRemovingWidget(false);
    }
  }, [pendingRemoveWidgetId, removeWidget]);

  useEffect(
    () => () => {
      if (reloadTimerRef.current !== null) {
        window.clearTimeout(reloadTimerRef.current);
      }
    },
    [],
  );

  useEffect(() => {
    if (externalReloadToken === 0) return;
    setFilesReloadToken((prev) => prev + 1);
    reload();
  }, [externalReloadToken, reload]);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    void fetchWidgetRegistry(orbitId)
      .then((next) => {
        if (!cancelled) setWidgets(next);
      })
      .catch((err) => {
        if (cancelled) return;
        const message = err instanceof Error ? err.message : String(err);
        setWidgets([]);
        onRuntimeNotice?.(message);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [orbitId, onRuntimeNotice, reloadToken]);

  useEffect(() => {
    const source = new EventSource(arkorbitApi.orbitEventsUrl(orbitId), {
      withCredentials: true,
    });
    const handleFileChanged = (event: MessageEvent) => {
      const path = orbitChangedPath(event);
      if (path) setFilesReloadToken((prev) => prev + 1);
      if (shouldReloadForPath(path)) reload();
    };
    source.addEventListener("file_changed", handleFileChanged as EventListener);
    source.onerror = () => {
      onRuntimeNotice?.("Orbit file event stream disconnected.");
    };
    return () => source.close();
  }, [orbitId, onRuntimeNotice, reload]);

  const widgetLayouts = useMemo(
    () =>
      resolveWidgetLayouts(
        orbitId,
        widgets,
        currentViewport(frameShellRef.current),
        measureMountedWidgetRects(frameShellRef.current),
      ),
    [orbitId, reloadToken, widgets],
  );

  return (
    <Box className="orbit-frame-shell" ref={frameShellRef}>
      {loading ? (
        <Typography variant="caption" className="orbit-frame-status">
          Loading orbit
        </Typography>
      ) : null}
      <OrbitFilesPanel
        orbitId={orbitId}
        reloadToken={filesReloadToken}
        onRuntimeNotice={onRuntimeNotice}
      />
      <Box className="orbit-frame-canvas">
        {widgets.length === 0 && !loading ? (
          <Box className="orbit-empty-canvas" aria-label="Empty Orbit canvas">
            <Box className="orbit-empty-topline">
              <span>Canvas</span>
              <span>Ready</span>
            </Box>
            <Box className="orbit-empty-reticle" aria-hidden="true" />
          </Box>
        ) : null}
        {widgets.map((widget, index) => (
          (() => {
            const identity = widgetIdentity(widget, index);
            if (!identity) return null;
            const fallbackSize = widgetSize(widget);
            const layout = widgetLayouts[identity.id] ?? {
              x: numberValue(widget.left, WIDGET_VIEWPORT_MARGIN),
              y: numberValue(widget.top, WIDGET_TOP_VIEWPORT_MARGIN),
              width: fallbackSize.width,
              height: fallbackSize.height,
            };
            return (
              <OrbitWidgetSlot
                key={`${widget.id || widget.module || index}:${reloadToken}`}
                orbitId={orbitId}
                widget={widget}
                index={index}
                layout={layout}
                reloadToken={reloadToken}
                onRemove={setPendingRemoveWidgetId}
                onMove={moveWidget}
                onRuntimeNotice={onRuntimeNotice}
              />
            );
          })()
        ))}
      </Box>
      <Dialog
        open={pendingRemoveWidgetId !== null}
        onClose={() => {
          if (!removingWidget) setPendingRemoveWidgetId(null);
        }}
        maxWidth="xs"
        fullWidth
      >
        <DialogTitle>Remove widget?</DialogTitle>
        <DialogContent>
          <Stack spacing={2} sx={{ pt: 0.5 }}>
            <Typography variant="body2">
              Remove <strong>{pendingRemoveWidget?.title ?? "this widget"}</strong> from this
              Orbit canvas?
            </Typography>
            <Alert severity="warning">
              This also deletes the widget code module if no other widget still uses it.
            </Alert>
          </Stack>
        </DialogContent>
        <DialogActions sx={{ px: 3, pb: 2 }}>
          <Button
            size="small"
            onClick={() => setPendingRemoveWidgetId(null)}
            disabled={removingWidget}
          >
            Cancel
          </Button>
          <Button
            size="small"
            color="error"
            variant="contained"
            onClick={confirmRemoveWidget}
            disabled={removingWidget || pendingRemoveWidgetId === null}
          >
            Remove widget
          </Button>
        </DialogActions>
      </Dialog>
    </Box>
  );
}

export default OrbitFrame;
