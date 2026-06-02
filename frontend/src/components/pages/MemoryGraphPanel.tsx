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
import ReactECharts from "echarts-for-react";
import { Check, RefreshCw, Search, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
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

type MemoryGraphPanelProps = {
  focusMemoryId?: string | null;
};

type ChartClickParams = {
  dataType?: string;
  data?: unknown;
};

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
  const inputSx = {
    height: 34,
    borderRadius: 1,
    border: "1px solid rgba(148, 163, 184, 0.26)",
    background: "rgba(15, 23, 32, 0.72)",
    color: "text.primary",
    px: 1,
    font: "inherit",
    fontSize: 13,
  } as const;

  const option = useMemo(() => {
    const categoriesForChart = [
      { name: "Memory" },
      { name: "Entity" },
      { name: "Source" },
    ];
    const data = nodes.map((node) => ({
      ...node,
      name: str(node.label, node.id),
      value: str(node.detail, ""),
      category: categoriesForChart.findIndex(
        (category) => category.name === graphNodeCategory(node),
      ),
      symbolSize: graphNodeSize(node),
      itemStyle: { color: graphNodeColor(node) },
      label: {
        show: Boolean(node.pinned || node.node_type === "entity"),
      },
    }));
    const links = edges.map((edge) => ({
      ...edge,
      source: edge.source,
      target: edge.target,
      value: memoryGraphEdgeLabel(edge),
      lineStyle: {
        color: edgeColor(edge),
        width: edge.edge_type === "knowledge_relation" ? 1.5 : 0.85,
        opacity: edge.semantic ? 0.32 : 0.46,
        curveness: edge.edge_type === "knowledge_relation" ? 0.12 : 0.04,
      },
    }));
    return {
      backgroundColor: "transparent",
      animationDurationUpdate: 350,
      tooltip: {
        trigger: "item",
        backgroundColor: "rgba(15, 18, 22, 0.96)",
        borderColor: "rgba(148, 163, 184, 0.24)",
        textStyle: { color: "#f8fafc", fontSize: 12 },
        formatter: graphTooltip,
      },
      legend: [
        {
          top: 2,
          right: 8,
          itemWidth: 8,
          itemHeight: 8,
          textStyle: { color: "#b8c3cf", fontSize: 11 },
          data: categoriesForChart.map((category) => category.name),
        },
      ],
      series: [
        {
          type: "graph",
          layout: "force",
          roam: true,
          draggable: false,
          top: 28,
          bottom: 8,
          left: 8,
          right: 8,
          scaleLimit: { min: 0.35, max: 5 },
          categories: categoriesForChart,
          data,
          links,
          label: {
            position: "right",
            color: "#edf2f7",
            fontSize: 10,
            overflow: "truncate",
            width: 160,
          },
          edgeLabel: { show: false },
          focusNodeAdjacency: true,
          force: {
            repulsion: 130,
            edgeLength: [58, 132],
            gravity: 0.06,
            friction: 0.58,
          },
          emphasis: {
            focus: "adjacency",
            label: { show: true },
            itemStyle: { borderColor: "#f8fafc", borderWidth: 1.5 },
            lineStyle: { opacity: 0.9, width: 1.6 },
          },
          blur: {
            itemStyle: { opacity: 0.28 },
            lineStyle: { opacity: 0.06 },
          },
          itemStyle: {
            borderColor: "#0f1720",
            borderWidth: 1.2,
          },
        },
      ],
    };
  }, [edges, nodes]);

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
              sx={{ ...inputSx, minWidth: 240 }}
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
              <ReactECharts
                option={option}
                style={{ height: "100%", width: "100%" }}
                notMerge
                lazyUpdate
                onEvents={{
                  click: (params: ChartClickParams) => {
                    const data = asRecord(params.data);
                    if (params.dataType === "edge") {
                      setSelectedEdge(data as MemoryGraphEdge);
                      setSelectedNode(null);
                    } else {
                      setSelectedNode(data as MemoryGraphNode);
                      setSelectedEdge(null);
                    }
                  },
                }}
              />
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
