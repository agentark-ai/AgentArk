export type MemoryGraphMode = "map" | "focus";

export type MemoryGraphQueryOptions = {
  mode: MemoryGraphMode;
  memoryId?: string;
  projectId?: string;
  limit?: number;
  categories?: string[];
  statuses?: string[];
  edgeTypes?: string[];
  relationStatuses?: string[];
  entityStatuses?: string[];
  relationTypes?: string[];
  sources?: string[];
  minConfidence?: number;
  updatedAfter?: string;
  updatedBefore?: string;
  semanticThreshold?: number;
  includeSemantic?: boolean;
};

export type MemoryGraphNode = {
  id: string;
  node_type?: string;
  label?: string;
  detail?: string;
  category?: string;
  status?: string;
  memory_kind?: string;
  confidence?: number;
  support_count?: number;
  stale?: boolean;
  pinned?: boolean;
  updated_at?: string;
  ref_kind?: string;
  metadata?: Record<string, unknown>;
};

export type MemoryGraphEdge = {
  id?: string;
  source?: string;
  target?: string;
  edge_type?: string;
  label?: string;
  detail?: string;
  weight?: number;
  semantic?: boolean;
  explicit?: boolean;
  updated_at?: string;
  metadata?: Record<string, unknown>;
};

export type MemoryGraphPayload = {
  mode?: MemoryGraphMode;
  nodes?: MemoryGraphNode[];
  edges?: MemoryGraphEdge[];
  truncated?: boolean;
  node_count?: number;
  edge_count?: number;
  semantic_edge_count?: number;
  knowledge_relation_count?: number;
};

export const MEMORY_GRAPH_MAX_LIMIT = 220;

function cleanTokens(values: string[] | undefined): string[] {
  const seen = new Set<string>();
  const tokens: string[] = [];
  for (const raw of values ?? []) {
    const token = String(raw || "").trim();
    if (!token || seen.has(token)) continue;
    seen.add(token);
    tokens.push(token);
  }
  return tokens;
}

function boundedGraphLimit(limit: number | undefined): number {
  if (!Number.isFinite(limit ?? NaN)) return 160;
  return Math.max(1, Math.min(MEMORY_GRAPH_MAX_LIMIT, Math.floor(limit as number)));
}

function thresholdToken(value: number | undefined): string | null {
  if (!Number.isFinite(value ?? NaN)) return null;
  const bounded = Math.max(0, Math.min(1, Number(value)));
  return String(Number(bounded.toFixed(3)));
}

export function buildMemoryGraphQuery(options: MemoryGraphQueryOptions): string {
  const params = new URLSearchParams();
  params.set("mode", options.mode);
  if (options.mode === "focus" && options.memoryId) {
    params.set("memory_id", options.memoryId);
  }
  if (options.projectId?.trim()) params.set("project_id", options.projectId.trim());
  params.set("limit", String(boundedGraphLimit(options.limit)));

  const categories = cleanTokens(options.categories);
  if (categories.length > 0) params.set("category", categories.join(","));

  const statuses = cleanTokens(options.statuses);
  if (statuses.length > 0) params.set("status", statuses.join(","));

  const edgeTypes = cleanTokens(options.edgeTypes);
  if (edgeTypes.length > 0) params.set("edge_type", edgeTypes.join(","));

  const relationStatuses = cleanTokens(options.relationStatuses);
  if (relationStatuses.length > 0) params.set("relation_status", relationStatuses.join(","));

  const entityStatuses = cleanTokens(options.entityStatuses);
  if (entityStatuses.length > 0) params.set("entity_status", entityStatuses.join(","));

  const relationTypes = cleanTokens(options.relationTypes);
  if (relationTypes.length > 0) params.set("relation_type", relationTypes.join(","));

  const sources = cleanTokens(options.sources);
  if (sources.length > 0) params.set("source", sources.join(","));

  if (Number.isFinite(options.minConfidence ?? NaN)) {
    params.set("min_confidence", String(Math.max(0, Math.min(1, Number(options.minConfidence))).toFixed(2)));
  }

  if (options.updatedAfter?.trim()) params.set("updated_after", options.updatedAfter.trim());
  if (options.updatedBefore?.trim()) params.set("updated_before", options.updatedBefore.trim());

  const semanticThreshold = thresholdToken(options.semanticThreshold);
  if (semanticThreshold) params.set("semantic_threshold", semanticThreshold);

  if (typeof options.includeSemantic === "boolean") {
    params.set("include_semantic", String(options.includeSemantic));
  }

  return `/arkmemory/graph?${params.toString()}`;
}

export function memoryGraphEdgeLabel(edge: Pick<MemoryGraphEdge, "edge_type" | "label" | "semantic">): string {
  if (edge.label?.trim()) return edge.label.trim();
  if (edge.semantic || edge.edge_type === "semantic_nearby") return "Semantic";
  const edgeType = String(edge.edge_type || "").trim();
  if (!edgeType) return "Link";
  return edgeType
    .split(/[_-]+/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(" ");
}

export function memoryGraphEdgeTone(edge: Pick<MemoryGraphEdge, "edge_type" | "semantic">): "semantic" | "supersedes" | "evidence" | "explicit" {
  const type = String(edge.edge_type || "").trim();
  if (edge.semantic || type === "semantic_nearby") return "semantic";
  if (type === "supersedes") return "supersedes";
  if (type === "evidence" || type === "operation" || type === "event") return "evidence";
  if (type === "knowledge_relation" || type === "relation_evidence") return "explicit";
  return "explicit";
}

export function memoryGraphVisibleSummary(payload: Pick<MemoryGraphPayload, "nodes" | "edges" | "truncated" | "semantic_edge_count" | "knowledge_relation_count">): string {
  const nodeCount = payload.nodes?.length ?? 0;
  const edgeCount = payload.edges?.length ?? 0;
  const semanticCount = payload.semantic_edge_count ?? 0;
  const relationCount = payload.knowledge_relation_count ?? 0;
  return [
    `${nodeCount} node${nodeCount === 1 ? "" : "s"}`,
    `${edgeCount} link${edgeCount === 1 ? "" : "s"}`,
    `${semanticCount} semantic`,
    relationCount > 0 ? `${relationCount} relation${relationCount === 1 ? "" : "s"}` : "",
    payload.truncated ? "capped" : "",
  ]
    .filter(Boolean)
    .join(", ");
}
