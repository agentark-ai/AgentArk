import {
  Alert,
  Box,
  Button,
  Chip,
  Divider,
  FormControlLabel,
  IconButton,
  Stack,
  Switch,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from "@mui/material";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { forceCollide, forceManyBody, forceX, forceY } from "d3-force";
import { Check, RefreshCw, Search, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import ForceGraph2D, {
  type ForceGraphMethods,
  type LinkObject,
  type NodeObject,
} from "react-force-graph-2d";
import { api } from "../../api/client";
import { humanizeMachineLabel } from "../../lib/displayLabels";
import {
  buildMemoryGraphQuery,
  memoryGraphEdgeLabel,
  memoryGraphEdgeTone,
  memoryGraphVisibleSummary,
  type MemoryGraphEdge,
  type MemoryGraphMode,
  type MemoryGraphNode,
  type MemoryGraphPayload,
} from "./memoryGraph";
import { asRecord, errMessage, num, str, type JsonRecord } from "./pageHelpers";

const GRAPH_MEMORY_STATUSES = ["active", "stale", "deprecated"];
const GRAPH_RELATION_STATUSES = ["candidate", "confirmed"];

const TWO_PI = Math.PI * 2;

// Node fields we derive once at merge time and stash on the simulation object so
// the per-frame paint never recomputes them.
type MemoryNodeExtra = MemoryGraphNode & {
  __color?: string;
  __r?: number;
  __label?: string;
  __degree?: number;
};

// The simulation augments these objects in place with x/y/vx/vy/fx/fy.
type GNode = NodeObject<MemoryNodeExtra>;
type GLink = LinkObject<MemoryNodeExtra, MemoryGraphEdge>;

type MemoryGraphPanelProps = {
  focusMemoryId?: string | null;
};

function clamp(value: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, value));
}

// A link endpoint is an id string before the first tick, a resolved node object after.
function idOf(endpoint: unknown): string {
  if (endpoint && typeof endpoint === "object") {
    const id = (endpoint as { id?: string | number }).id;
    return id == null ? "" : String(id);
  }
  return endpoint == null ? "" : String(endpoint);
}

function graphNodeCategory(node: MemoryGraphNode): string {
  const type = str(node.node_type, "memory");
  if (type === "entity") return "Entity";
  if (type === "source") return "Source";
  return "Memory";
}

function graphNodeSize(node: MemoryGraphNode): number {
  if (node.pinned) return 30;
  if (node.node_type === "entity") return 24;
  if (node.node_type === "source") return 14;
  const confidence = num(node.confidence, 0.7);
  const support = Math.min(5, Math.max(0, num(node.support_count, 0)));
  return 15 + confidence * 8 + support;
}

// Convert the legacy symbol diameter into a force-graph world-unit radius.
function nodeRadius(node: MemoryGraphNode): number {
  return graphNodeSize(node) / 2.6;
}

function graphNodeColor(node: MemoryGraphNode): string {
  if (node.pinned) return "#f25555";
  if (node.node_type === "entity") return "#25b99a";
  if (node.node_type === "source") return "#a8b0b8";
  const category = str(node.category, "");
  if (category === "assistant_preference") return "#f4728f";
  if (category === "work_preference") return "#4b91e2";
  if (category === "project_domain_memory") return "#d49b0b";
  if (category === "ephemeral_context") return "#f3dc4d";
  if (category === "knowledge") return "#8b5cf6";
  if (category === "other") return "#f9733f";
  return "#6ea8ff";
}

function edgeColor(edge: MemoryGraphEdge): string {
  const tone = memoryGraphEdgeTone(edge);
  if (tone === "semantic") return "rgba(243, 220, 77, 0.42)";
  if (tone === "supersedes") return "rgba(242, 85, 85, 0.48)";
  if (tone === "evidence") return "rgba(168, 176, 184, 0.34)";
  if (edge.edge_type === "knowledge_relation") return "rgba(37, 185, 154, 0.56)";
  return "rgba(110, 168, 255, 0.38)";
}

function graphTooltip(params: { dataType?: string; data?: unknown }): string {
  const data = asRecord(params.data);
  if (params.dataType === "edge") {
    return [
      `<strong>${memoryGraphEdgeLabel(data as MemoryGraphEdge)}</strong>`,
      str(data.detail, ""),
      str(data.edge_type, ""),
    ]
      .filter(Boolean)
      .join("<br/>");
  }
  return [
    `<strong>${str(data.name || data.label, "Node")}</strong>`,
    humanizeMachineLabel(str(data.node_type, "memory"), "Memory"),
    str(data.value || data.detail, ""),
  ]
    .filter(Boolean)
    .join("<br/>");
}

function inspectorEvidence(edge: MemoryGraphEdge): JsonRecord[] {
  const metadata = asRecord(edge.metadata);
  const values = Array.isArray(metadata.evidence) ? metadata.evidence : [];
  return values.map(asRecord).filter((item) => Object.keys(item).length > 0);
}

const GRAPH_LEGEND: Array<{ label: string; color: string }> = [
  { label: "Memory", color: "#6ea8ff" },
  { label: "Entity", color: "#25b99a" },
  { label: "Source", color: "#a8b0b8" },
];

export default function MemoryGraphPanel({ focusMemoryId }: MemoryGraphPanelProps) {
  const queryClient = useQueryClient();
  const [mode, setMode] = useState<MemoryGraphMode>(focusMemoryId ? "focus" : "map");
  const [focusId, setFocusId] = useState(focusMemoryId || "");
  const [includeSemantic, setIncludeSemantic] = useState(true);
  const [selectedNode, setSelectedNode] = useState<MemoryGraphNode | null>(null);
  const [selectedEdge, setSelectedEdge] = useState<MemoryGraphEdge | null>(null);

  useEffect(() => {
    if (!focusMemoryId) return;
    setFocusId(focusMemoryId);
    setMode("focus");
  }, [focusMemoryId]);

  const queryPath = useMemo(
    () =>
      buildMemoryGraphQuery({
        mode,
        memoryId: focusId,
        limit: 160,
        statuses: GRAPH_MEMORY_STATUSES,
        relationStatuses: GRAPH_RELATION_STATUSES,
        includeSemantic,
        semanticThreshold: 0.78,
      }),
    [
      focusId,
      includeSemantic,
      mode,
    ],
  );

  const graphQ = useQuery({
    queryKey: ["arkmemory-graph", queryPath],
    queryFn: () => api.rawGet(queryPath) as Promise<MemoryGraphPayload>,
    enabled: mode === "map" || focusId.trim().length > 0,
    staleTime: 15_000,
  });

  const relationStatusMutation = useMutation({
    mutationFn: ({ id, action }: { id: string; action: "confirm" | "reject" }) =>
      api.rawPost(
        `/arkmemory/knowledge-graph/relations/${encodeURIComponent(id)}/${action}`,
      ),
    onSuccess: async () => {
      await queryClient.invalidateQueries({ queryKey: ["arkmemory-graph"] });
    },
  });

  const payload = (graphQ.data || {}) as MemoryGraphPayload;
  const nodes = payload.nodes || [];
  const edges = payload.edges || [];
  const summary = memoryGraphVisibleSummary(payload);

  // --- Force-graph plumbing (refs so hover/selection never trigger React re-renders) ---
  const fgRef = useRef<ForceGraphMethods<GNode, GLink> | undefined>(undefined);
  const observerRef = useRef<ResizeObserver | null>(null);
  const [size, setSize] = useState({ width: 0, height: 0 });

  const nodesByIdRef = useRef<Map<string, GNode>>(new Map());
  const neighborsRef = useRef<Map<string, Set<string>>>(new Map());
  const linksByNodeRef = useRef<Map<string, Set<GLink>>>(new Map());
  const hoverRef = useRef<string | null>(null);
  const selectedIdRef = useRef<string | null>(null);
  const highlightNodesRef = useRef<Set<string>>(new Set());
  const highlightLinksRef = useRef<Set<GLink>>(new Set());
  const didFitRef = useRef(false);
  const prevIdsKeyRef = useRef("");
  const prevEdgeKeyRef = useRef("");
  const lastGraphDataRef = useRef<{ nodes: GNode[]; links: GLink[] } | null>(null);

  // Keep the paint callback's view of the selection current without re-creating it.
  useEffect(() => {
    selectedIdRef.current = selectedNode?.id ?? null;
  }, [selectedNode]);

  // Build graphData by MERGING into the existing node objects: the engine mutates
  // x/y/vx/vy in place and diffs by identity, so reusing objects keeps positions
  // stable across refetches instead of re-laying-out the whole graph every poll.
  const graphData = useMemo(() => {
    const byId = nodesByIdRef.current;
    const incomingIds = new Set<string>();

    const degree = new Map<string, number>();
    const links: GLink[] = [];
    for (const edge of edges) {
      const s = edge.source;
      const t = edge.target;
      if (!s || !t) continue;
      degree.set(s, (degree.get(s) ?? 0) + 1);
      degree.set(t, (degree.get(t) ?? 0) + 1);
      links.push({ ...edge } as GLink);
    }

    const outNodes: GNode[] = [];
    for (const node of nodes) {
      incomingIds.add(node.id);
      let obj = byId.get(node.id);
      if (obj) {
        Object.assign(obj, node); // refresh payload fields, preserve x/y/vx/vy
      } else {
        obj = { ...node } as GNode;
        byId.set(node.id, obj);
      }
      const rawLabel = str(node.label, node.id);
      obj.__color = graphNodeColor(node);
      obj.__r = nodeRadius(node);
      // Cap the on-canvas label length (the previous renderer truncated to ~160px).
      // The full text still shows in the hover tooltip and the inspector panel.
      obj.__label = rawLabel.length > 30 ? `${rawLabel.slice(0, 29)}…` : rawLabel;
      obj.__degree = degree.get(node.id) ?? 0;
      outNodes.push(obj);
    }
    // Drop nodes that left the result so the cache can't grow unbounded or revive stale positions.
    for (const id of Array.from(byId.keys())) {
      if (!incomingIds.has(id)) byId.delete(id);
    }

    const idKey = outNodes
      .map((n) => String(n.id))
      .sort()
      .join("|");
    const edgeKey = links
      .map((l) => `${idOf(l.source)}>${idOf(l.target)}:${str(l.edge_type, "")}`)
      .sort()
      .join("|");

    // Same node + edge set as last time (a field-only refresh): return the SAME
    // graphData reference so the engine does not re-heat — no jitter, the camera
    // holds, and the existing adjacency/highlight Sets stay valid (they key links
    // by identity). The node objects were already refreshed in place above.
    if (
      lastGraphDataRef.current &&
      idKey === prevIdsKeyRef.current &&
      edgeKey === prevEdgeKeyRef.current
    ) {
      return lastGraphDataRef.current;
    }

    // Structural change (mode/focus switch, added/removed memory or relation):
    // rebuild adjacency, allow exactly one re-fit, and publish a fresh reference
    // (which intentionally lets the engine re-heat and re-settle).
    const neighbors = new Map<string, Set<string>>();
    const linksByNode = new Map<string, Set<GLink>>();
    const bucket = <V,>(map: Map<string, Set<V>>, key: string): Set<V> => {
      let set = map.get(key);
      if (!set) {
        set = new Set<V>();
        map.set(key, set);
      }
      return set;
    };
    for (const link of links) {
      const s = idOf(link.source);
      const t = idOf(link.target);
      if (!s || !t) continue;
      bucket(neighbors, s).add(t);
      bucket(neighbors, t).add(s);
      bucket(linksByNode, s).add(link);
      bucket(linksByNode, t).add(link);
    }
    neighborsRef.current = neighbors;
    linksByNodeRef.current = linksByNode;

    prevIdsKeyRef.current = idKey;
    prevEdgeKeyRef.current = edgeKey;
    didFitRef.current = false;

    const next = { nodes: outNodes, links };
    lastGraphDataRef.current = next;
    return next;
  }, [nodes, edges]);

  // Callback ref: (re)attach the ResizeObserver every time the canvas wrapper
  // mounts. It unmounts on the empty state, so a one-time effect would observe a
  // detached node forever and never re-observe the remounted div. The canvas needs
  // explicit pixel width/height — it does not auto-fit its parent.
  const setWrap = useCallback((el: HTMLDivElement | null) => {
    observerRef.current?.disconnect();
    if (!el) {
      observerRef.current = null;
      setSize({ width: 0, height: 0 });
      return;
    }
    const observer = new ResizeObserver(([entry]) => {
      const { width, height } = entry.contentRect;
      setSize((prev) =>
        prev.width === width && prev.height === height ? prev : { width, height },
      );
    });
    observer.observe(el);
    observerRef.current = observer;
  }, []);

  // Configure the simulation once the imperative handle exists (re-applied on resize).
  useEffect(() => {
    const fg = fgRef.current;
    if (!fg) return;
    const setForce = fg.d3Force.bind(fg) as (name: string, force?: unknown) => unknown;
    // Repulsion is the spread driver; cap its range so far nodes stay cheap.
    setForce("charge", forceManyBody<GNode>().strength(-140).distanceMax(500).theta(0.9));
    // Keep d3's degree-aware default link strength (1/min(deg)) so hubs don't explode — only set distance.
    const linkForce = fg.d3Force("link") as { distance?: (d: number) => unknown } | undefined;
    linkForce?.distance?.(48);
    // Positioning forces toward origin (viewport centre) give a bounded, airy cloud.
    setForce("center", null);
    setForce("x", forceX<GNode>(0).strength(0.06));
    setForce("y", forceY<GNode>(0).strength(0.06));
    setForce(
      "collide",
      forceCollide<GNode>()
        .radius((n) => (n.__r ?? 6) + 2)
        .strength(0.85),
    );
    // The canvas dimensions changed, so re-fit once after this reheat settles
    // (onEngineStop is otherwise guarded against re-fitting and would leave the
    // graph framed to the old size).
    didFitRef.current = false;
    fg.d3ReheatSimulation();
  }, [size.width, size.height]);

  // Pause the render loop on unmount so it can't leak a rAF after route change.
  useEffect(() => {
    return () => {
      (fgRef.current as { pauseAnimation?: () => void } | undefined)?.pauseAnimation?.();
    };
  }, []);

  const paintNode = useCallback(
    (node: GNode, ctx: CanvasRenderingContext2D, globalScale: number) => {
      const x = node.x ?? 0;
      const y = node.y ?? 0;
      const r = node.__r ?? 6;
      const baseColor = node.__color ?? "#6ea8ff";
      const selId = selectedIdRef.current;
      const isSel = selId != null && String(node.id) === selId;
      const hovActive = hoverRef.current != null;
      const inHot = highlightNodesRef.current.has(String(node.id));
      // The active selection stays fully lit even while hovering an unrelated node.
      const isHot = !hovActive || inHot || isSel;

      ctx.save();
      ctx.globalAlpha = isHot ? 1 : 0.12;
      const glow = hovActive && inHot;
      if (glow) {
        ctx.shadowColor = baseColor;
        ctx.shadowBlur = (isSel ? 14 : 9) / globalScale;
      }
      ctx.beginPath();
      ctx.arc(x, y, r, 0, TWO_PI);
      ctx.fillStyle = baseColor;
      ctx.fill();
      // Always clear the shadow before stroke/label so it can't bleed into the next node.
      ctx.shadowBlur = 0;
      ctx.shadowColor = "transparent";
      ctx.lineWidth = 1.2 / globalScale;
      ctx.strokeStyle = "rgba(15,23,32,0.9)";
      ctx.stroke();

      if (isSel) {
        ctx.beginPath();
        ctx.arc(x, y, r * 1.5, 0, TWO_PI);
        ctx.lineWidth = 1.5 / globalScale;
        ctx.strokeStyle = "#78f2b0";
        ctx.stroke();
      }

      const deg = node.__degree ?? 0;
      const degreeBoost = Math.min(deg / 10, 1);
      const effThreshold = 1.6 * (1 - 0.6 * degreeBoost); // hubs label sooner
      const alwaysLabel = Boolean(node.pinned) || node.node_type === "entity";
      let labelAlpha: number;
      if (isSel || (isHot && hovActive)) labelAlpha = 1;
      else if (alwaysLabel) labelAlpha = 1;
      else labelAlpha = clamp((globalScale - effThreshold) / 0.8, 0, 1);

      if (labelAlpha > 0.02 && isHot && node.__label) {
        const fontSize = 11 / globalScale;
        ctx.font = `${fontSize}px Inter, system-ui, sans-serif`;
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        ctx.globalAlpha = labelAlpha;
        ctx.fillStyle = "#edf2f7";
        ctx.fillText(node.__label, x, y + r + 2 / globalScale);
      }
      ctx.restore();
    },
    [],
  );

  const paintPointerArea = useCallback(
    (node: GNode, color: string, ctx: CanvasRenderingContext2D) => {
      const x = node.x ?? 0;
      const y = node.y ?? 0;
      const r = (node.__r ?? 6) + 2;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.arc(x, y, r, 0, TWO_PI);
      ctx.fill();
    },
    [],
  );

  const linkColorFn = useCallback((link: GLink) => {
    if (hoverRef.current != null && !highlightLinksRef.current.has(link)) {
      return "rgba(140,160,200,0.06)";
    }
    return edgeColor(link as unknown as MemoryGraphEdge);
  }, []);

  const linkWidthFn = useCallback((link: GLink) => {
    const base = link.edge_type === "knowledge_relation" ? 1.5 : 0.85;
    return highlightLinksRef.current.has(link) ? base + 1.5 : base;
  }, []);

  const linkCurvatureFn = useCallback(
    (link: GLink) => (link.edge_type === "knowledge_relation" ? 0.12 : 0.04),
    [],
  );

  const handleNodeHover = useCallback((node: GNode | null) => {
    const hot = highlightNodesRef.current;
    const hotLinks = highlightLinksRef.current;
    hot.clear();
    hotLinks.clear();
    if (node && node.id != null) {
      const id = String(node.id);
      hot.add(id);
      neighborsRef.current.get(id)?.forEach((n) => hot.add(n));
      linksByNodeRef.current.get(id)?.forEach((l) => hotLinks.add(l));
      hoverRef.current = id;
    } else {
      hoverRef.current = null;
    }
  }, []);

  const handleNodeClick = useCallback((node: GNode) => {
    setSelectedNode(node as unknown as MemoryGraphNode);
    setSelectedEdge(null);
  }, []);

  const handleLinkClick = useCallback((link: GLink) => {
    setSelectedEdge(link as unknown as MemoryGraphEdge);
    setSelectedNode(null);
  }, []);

  // Release the drag pin so the node eases back into a physics-natural spot.
  const handleNodeDragEnd = useCallback((node: GNode) => {
    node.fx = undefined;
    node.fy = undefined;
  }, []);

  const handleEngineStop = useCallback(() => {
    if (!didFitRef.current) {
      didFitRef.current = true;
      fgRef.current?.zoomToFit(400, 40);
    }
  }, []);

  const relationId = str(asRecord(selectedEdge?.metadata).relation_id, "");
  const evidence = selectedEdge ? inspectorEvidence(selectedEdge) : [];

  return (
    <Box className="list-shell">
      <Stack spacing={1.25}>
        <Stack
          direction="row"
          spacing={1}
          useFlexGap
          sx={{ alignItems: "center", flexWrap: "wrap" }}
        >
          <ToggleButtonGroup
            exclusive
            size="small"
            value={mode}
            onChange={(_event, next) => {
              if (next === "map" || next === "focus") setMode(next);
            }}
          >
            <ToggleButton value="map">All</ToggleButton>
            <ToggleButton value="focus">Selected</ToggleButton>
          </ToggleButtonGroup>
          {mode === "focus" ? (
            <Box
              component="input"
              value={focusId}
              onChange={(event) => setFocusId(event.currentTarget.value)}
              placeholder="Paste memory id"
              sx={{
                height: 34,
                borderRadius: 1,
                border: "1px solid rgba(148, 163, 184, 0.26)",
                background: "rgba(15, 23, 32, 0.72)",
                color: "text.primary",
                px: 1,
                font: "inherit",
                fontSize: 13,
                minWidth: 240,
              }}
            />
          ) : null}
          <Tooltip title="Show embedding-nearby memories">
            <FormControlLabel
              control={
                <Switch
                  size="small"
                  checked={includeSemantic}
                  onChange={(event) => setIncludeSemantic(event.currentTarget.checked)}
                />
              }
              label="Nearby"
            />
          </Tooltip>
          <Tooltip title="Refresh graph">
            <IconButton size="small" onClick={() => graphQ.refetch()} disabled={graphQ.isFetching}>
              <RefreshCw size={17} />
            </IconButton>
          </Tooltip>
          <Chip size="small" variant="outlined" label={summary} />
        </Stack>

        {graphQ.error ? <Alert severity="error">{errMessage(graphQ.error)}</Alert> : null}

        <Stack direction={{ xs: "column", lg: "row" }} spacing={1.25}>
          <Box
            sx={{
              position: "relative",
              minHeight: { xs: 520, lg: 640 },
              height: { xs: 520, lg: 640 },
              flex: 1,
              border: "1px solid rgba(148, 163, 184, 0.16)",
              borderRadius: 1,
              background: "rgba(10, 14, 18, 0.72)",
              overflow: "hidden",
            }}
          >
            {nodes.length === 0 && !graphQ.isFetching ? (
              <Stack
                sx={{
                  height: "100%",
                  alignItems: "center",
                  justifyContent: "center",
                  color: "text.secondary",
                }}
                spacing={1}
              >
                <Search size={22} />
                <Typography variant="body2">No memories to show yet.</Typography>
              </Stack>
            ) : (
              <div ref={setWrap} style={{ position: "absolute", inset: 0 }}>
                <Stack
                  direction="row"
                  spacing={1.25}
                  sx={{
                    position: "absolute",
                    top: 8,
                    right: 12,
                    zIndex: 2,
                    pointerEvents: "none",
                    alignItems: "center",
                  }}
                >
                  {GRAPH_LEGEND.map((entry) => (
                    <Stack
                      key={entry.label}
                      direction="row"
                      spacing={0.5}
                      sx={{ alignItems: "center" }}
                    >
                      <Box
                        sx={{
                          width: 8,
                          height: 8,
                          borderRadius: "50%",
                          background: entry.color,
                        }}
                      />
                      <Typography variant="caption" sx={{ color: "#b8c3cf", fontSize: 11 }}>
                        {entry.label}
                      </Typography>
                    </Stack>
                  ))}
                </Stack>
                {size.width > 0 && size.height > 0 ? (
                  <ForceGraph2D<MemoryNodeExtra, MemoryGraphEdge>
                    ref={fgRef}
                    width={size.width}
                    height={size.height}
                    graphData={graphData}
                    backgroundColor="rgba(0,0,0,0)"
                    nodeCanvasObject={paintNode}
                    nodeCanvasObjectMode={() => "replace"}
                    nodePointerAreaPaint={paintPointerArea}
                    nodeLabel={(node: GNode) => graphTooltip({ data: node })}
                    linkLabel={(link: GLink) => graphTooltip({ dataType: "edge", data: link })}
                    linkColor={linkColorFn}
                    linkWidth={linkWidthFn}
                    linkCurvature={linkCurvatureFn}
                    linkDirectionalParticles={0}
                    onNodeHover={handleNodeHover}
                    onNodeClick={handleNodeClick}
                    onLinkClick={handleLinkClick}
                    onNodeDragEnd={handleNodeDragEnd}
                    onEngineStop={handleEngineStop}
                    cooldownTicks={120}
                    d3VelocityDecay={0.4}
                    d3AlphaDecay={0.0182}
                    minZoom={0.1}
                    maxZoom={8}
                    autoPauseRedraw={false}
                  />
                ) : null}
              </div>
            )}
          </Box>

          <Box
            sx={{
              width: { xs: "100%", lg: 340 },
              border: "1px solid rgba(148, 163, 184, 0.16)",
              borderRadius: 1,
              p: 1.25,
              alignSelf: "stretch",
              background: "rgba(15, 23, 32, 0.52)",
            }}
          >
            {selectedEdge ? (
              <Stack spacing={1}>
                <Stack direction="row" spacing={0.75} sx={{ alignItems: "center" }}>
                  <Typography variant="subtitle2" sx={{ fontWeight: 700, flex: 1 }}>
                    {memoryGraphEdgeLabel(selectedEdge)}
                  </Typography>
                  <IconButton size="small" onClick={() => setSelectedEdge(null)}>
                    <X size={15} />
                  </IconButton>
                </Stack>
                <Chip
                  size="small"
                  variant="outlined"
                  label={humanizeMachineLabel(str(selectedEdge.edge_type, "link"))}
                  sx={{ alignSelf: "flex-start" }}
                />
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  {str(selectedEdge.detail, "No detail recorded.")}
                </Typography>
                {relationId ? (
                  <Stack direction="row" spacing={0.75}>
                    <Button
                      size="small"
                      variant="contained"
                      startIcon={<Check size={15} />}
                      disabled={relationStatusMutation.isPending}
                      onClick={() =>
                        relationStatusMutation.mutate({
                          id: relationId,
                          action: "confirm",
                        })
                      }
                    >
                      Confirm
                    </Button>
                    <Button
                      size="small"
                      color="warning"
                      variant="outlined"
                      disabled={relationStatusMutation.isPending}
                      onClick={() =>
                        relationStatusMutation.mutate({
                          id: relationId,
                          action: "reject",
                        })
                      }
                    >
                      Reject
                    </Button>
                  </Stack>
                ) : null}
                {evidence.length > 0 ? (
                  <>
                    <Divider />
                    <Stack spacing={0.75}>
                      {evidence.map((item, index) => (
                        <Box key={`${str(item.id, "evidence")}-${index}`}>
                          <Typography variant="caption" sx={{ color: "text.secondary" }}>
                            {humanizeMachineLabel(str(item.evidence_kind, "evidence"))}
                          </Typography>
                          <Typography variant="body2">
                            {str(item.excerpt, str(item.evidence_ref, ""))}
                          </Typography>
                        </Box>
                      ))}
                    </Stack>
                  </>
                ) : null}
              </Stack>
            ) : selectedNode ? (
              <Stack spacing={1}>
                <Stack direction="row" spacing={0.75} sx={{ alignItems: "center" }}>
                  <Typography variant="subtitle2" sx={{ fontWeight: 700, flex: 1 }}>
                    {str(selectedNode.label, selectedNode.id)}
                  </Typography>
                  <IconButton size="small" onClick={() => setSelectedNode(null)}>
                    <X size={15} />
                  </IconButton>
                </Stack>
                <Stack direction="row" spacing={0.75} useFlexGap sx={{ flexWrap: "wrap" }}>
                  <Chip
                    size="small"
                    variant="outlined"
                    label={humanizeMachineLabel(str(selectedNode.node_type, "memory"))}
                  />
                  {selectedNode.status ? (
                    <Chip
                      size="small"
                      variant="outlined"
                      label={humanizeMachineLabel(selectedNode.status)}
                    />
                  ) : null}
                </Stack>
                <Typography variant="body2" sx={{ color: "text.secondary" }}>
                  {str(selectedNode.detail, "No detail recorded.")}
                </Typography>
                <Typography
                  variant="caption"
                  sx={{ color: "text.secondary", overflowWrap: "anywhere" }}
                >
                  {selectedNode.id}
                </Typography>
              </Stack>
            ) : (
              <Stack spacing={1} sx={{ color: "text.secondary" }}>
                <Typography variant="subtitle2" sx={{ color: "text.primary" }}>
                  Selection
                </Typography>
                <Typography variant="body2">No selection.</Typography>
              </Stack>
            )}
          </Box>
        </Stack>
      </Stack>
    </Box>
  );
}
