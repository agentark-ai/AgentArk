import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-evolution-review-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/evolutionReviewQueueHelpers.ts",
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

const compiledHelperCandidates = [
  path.join(outDir, "evolutionReviewQueueHelpers.js"),
  path.join(outDir, "components", "pages", "evolutionReviewQueueHelpers.js"),
];
const compiledHelper = compiledHelperCandidates.find((candidate) => existsSync(candidate));

assert.ok(compiledHelper, "compiled helper output should exist");

const { promptHoldoutFootprintRows } = await import(pathToFileURL(compiledHelper).toString());

test("identical prompt holdout footprints collapse into one readable row", () => {
  const rows = promptHoldoutFootprintRows([
    {
      outcome: "slow",
      trace_id: "trace-a",
      section_chars: 22956,
      final_prompt_chars: 24738,
    },
    {
      outcome: "slow",
      trace_id: "trace-b",
      section_chars: 22956,
      final_prompt_chars: 24738,
    },
    {
      outcome: "slow",
      trace_id: "trace-c",
      section_chars: 22956,
      final_prompt_chars: 24738,
    },
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].label, "Slow cases (3)");
  assert.equal(rows[0].targetChars, 22956);
  assert.equal(rows[0].restChars, 1782);
  assert.equal(rows[0].totalChars, 24738);
});

test("backend matching sample counts are preserved for representative holdout rows", () => {
  const rows = promptHoldoutFootprintRows([
    {
      outcome: "slow",
      trace_id: "trace-a",
      matching_samples: 5,
      section_chars: 22956,
      final_prompt_chars: 24738,
    },
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].label, "Slow cases (5)");
  assert.equal(rows[0].count, 5);
});

test("distinct prompt holdout footprints keep separate readable rows", () => {
  const rows = promptHoldoutFootprintRows([
    {
      outcome: "slow",
      trace_id: "trace-a",
      section_chars: 22000,
      final_prompt_chars: 25000,
    },
    {
      outcome: "expensive",
      trace_id: "trace-b",
      section_chars: 18000,
      final_prompt_chars: 26000,
    },
  ]);

  assert.deepEqual(
    rows.map((row) => row.label),
    ["Slow case 1", "Expensive case 2"],
  );
});
