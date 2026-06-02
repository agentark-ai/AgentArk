import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-memory-graph-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/memoryGraph.ts",
    "--ignoreConfig",
    "--target",
    "ES2020",
    "--module",
    "ES2020",
    "--moduleResolution",
    "Bundler",
    "--outDir",
    outDir,
    "--skipLibCheck",
  ],
  { cwd: frontendRoot, stdio: "inherit" },
);
writeFileSync(path.join(outDir, "package.json"), JSON.stringify({ type: "module" }));

const {
  buildMemoryGraphQuery,
  memoryGraphEdgeLabel,
  memoryGraphEdgeTone,
  memoryGraphVisibleSummary,
} = await import(pathToFileURL(path.join(outDir, "memoryGraph.js")).toString());

test("buildMemoryGraphQuery keeps map requests bounded and structured", () => {
  assert.equal(
    buildMemoryGraphQuery({
      mode: "map",
      projectId: "project-alpha",
      limit: 999,
      categories: ["profile_fact", "work_preference"],
      statuses: ["active", "deprecated"],
      edgeTypes: ["supports", "semantic_nearby"],
      relationStatuses: ["candidate", "confirmed"],
      semanticThreshold: 0.72,
      includeSemantic: true,
    }),
    "/arkmemory/graph?mode=map&project_id=project-alpha&limit=220&category=profile_fact%2Cwork_preference&status=active%2Cdeprecated&edge_type=supports%2Csemantic_nearby&relation_status=candidate%2Cconfirmed&semantic_threshold=0.72&include_semantic=true",
  );
});

test("buildMemoryGraphQuery encodes focused graph memory ids", () => {
  assert.equal(
    buildMemoryGraphQuery({
      mode: "focus",
      memoryId: "memory/id with spaces",
      limit: 50,
      includeSemantic: true,
    }),
    "/arkmemory/graph?mode=focus&memory_id=memory%2Fid+with+spaces&limit=50&include_semantic=true",
  );
});

test("memory graph edge helpers distinguish explicit and semantic links", () => {
  assert.equal(memoryGraphEdgeLabel({ edge_type: "semantic_nearby", semantic: true }), "Semantic");
  assert.equal(memoryGraphEdgeTone({ edge_type: "semantic_nearby", semantic: true }), "semantic");
  assert.equal(memoryGraphEdgeLabel({ edge_type: "supersedes", semantic: false }), "Supersedes");
});

test("memoryGraphVisibleSummary reflects filtered visible graph size", () => {
  assert.equal(
    memoryGraphVisibleSummary({
      nodes: [{ id: "a" }, { id: "b" }, { id: "c" }],
      edges: [{ id: "e1" }, { id: "e2" }],
      truncated: true,
      semantic_edge_count: 1,
      knowledge_relation_count: 1,
    }),
    "3 nodes, 2 links, 1 semantic, 1 relation, capped",
  );
});
