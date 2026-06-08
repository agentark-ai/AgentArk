import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-reflect-opportunities-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/arkReflectOpportunityHelpers.ts",
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
  followupHasSourceEvidence,
  isDisplayableOpportunity,
  isOpportunitySettlementActive,
  latestUpdateTitle,
  latestDevelopmentSummary,
  shouldStartOpportunitySettlementPoll,
  shouldPollForOpportunitySettlement,
} = await import(pathToFileURL(path.join(outDir, "arkReflectOpportunityHelpers.js")).toString());

function followup(overrides = {}) {
  return {
    id: "followup",
    kind: "latest_developments",
    title: "search india iran news. Here's the latest on India and the Iran conflict, based on what's unfolding as of today",
    detail: "Reflect found a user-authored topic that may benefit from current public sources. Source enrichment is queued.",
    status: "queued",
    search_results: [],
    latest_summary: null,
    latest_summary_error: null,
    search_error: null,
    latest_summary_evidence_supported: null,
    ...overrides,
  };
}

test("planned source checks are displayable while evidence is pending", () => {
  const item = followup();

  assert.equal(followupHasSourceEvidence(item), false);
  assert.equal(isDisplayableOpportunity(item), true);
});

test("source-backed opportunity titles come from source documents instead of raw chat text", () => {
  const item = followup({
    status: "ready",
    search_results: [
      {
        title: "India Today indiatoday.in > news > topic > iran Iran Latest News, Iran Top Stories",
        url: "https://www.indiatoday.in/topic/iran",
        snippet: "Iran Latest News, Iran Top Stories, Updates, Photos, Videos.",
        source: "India Today",
        published_date: "2026-06-01",
      },
    ],
    latest_summary: "Source check found 1 cached source for India Iran conflict.",
    latest_summary_evidence_supported: true,
  });

  assert.equal(isDisplayableOpportunity(item), true);
  assert.match(latestUpdateTitle(item), /Iran Latest News/i);
  assert.doesNotMatch(latestUpdateTitle(item), /^search india iran news/i);
});

test("unsupported source checks stay visible as planned opportunities", () => {
  const item = followup({
    status: "ready",
    search_results: [
      {
        title: "Broad topic reference",
        url: "https://example.com/reference",
        snippet: "General background about one entity.",
        source: "Example",
        published_date: "2026-06-03",
      },
    ],
    latest_summary: null,
    latest_summary_error: "Source snippets do not directly support the requested topic.",
    latest_summary_evidence_supported: false,
  });

  assert.equal(followupHasSourceEvidence(item), false);
  assert.equal(isDisplayableOpportunity(item), true);
});

test("source summary explains cached sources when synthesis failed", () => {
  const item = followup({
    status: "ready",
    latest_summary_error: "Latest-development summary timed out.",
    search_results: [
      {
        title: "Diplomatic position",
        url: "https://www.dw.com/example",
        snippet: "Iran war tests India's multi-alignment diplomacy.",
        source: "DW",
        published_date: null,
      },
    ],
  });

  assert.match(latestDevelopmentSummary(item), /cached source/i);
  assert.doesNotMatch(latestDevelopmentSummary(item), /pending/i);
});

test("activity without displayable opportunities keeps polling during enrichment", () => {
  assert.equal(
    shouldPollForOpportunitySettlement({
      sourceCounts: { main_chat: 2, memory: 0 },
      opportunityCount: 0,
      queuedSourceCheckCount: 0,
      refreshRunning: false,
    }),
    true,
  );
});

test("opportunity settlement polling stops when already handled elsewhere", () => {
  assert.equal(
    shouldPollForOpportunitySettlement({
      sourceCounts: { main_chat: 2 },
      opportunityCount: 1,
      queuedSourceCheckCount: 0,
      refreshRunning: false,
    }),
    false,
  );
  assert.equal(
    shouldPollForOpportunitySettlement({
      sourceCounts: { main_chat: 2 },
      opportunityCount: 1,
      queuedSourceCheckCount: 1,
      refreshRunning: false,
    }),
    true,
  );
  assert.equal(
    shouldPollForOpportunitySettlement({
      sourceCounts: { main_chat: 2 },
      opportunityCount: 0,
      queuedSourceCheckCount: 0,
      refreshRunning: true,
    }),
    false,
  );
  assert.equal(
    shouldPollForOpportunitySettlement({
      sourceCounts: { main_chat: 0, memory: 0 },
      opportunityCount: 0,
      queuedSourceCheckCount: 0,
      refreshRunning: false,
    }),
    false,
  );
});

test("opportunity settlement polling does not restart after the same recap expires", () => {
  const now = 1000;

  assert.equal(
    shouldStartOpportunitySettlementPoll({
      shouldPoll: true,
      currentUntil: undefined,
      now,
    }),
    true,
  );
  assert.equal(
    isOpportunitySettlementActive({
      shouldPoll: true,
      currentUntil: now + 5000,
      now,
    }),
    true,
  );
  assert.equal(
    shouldStartOpportunitySettlementPoll({
      shouldPoll: true,
      currentUntil: now - 1,
      now,
    }),
    false,
  );
  assert.equal(
    isOpportunitySettlementActive({
      shouldPoll: true,
      currentUntil: now - 1,
      now,
    }),
    false,
  );
});
