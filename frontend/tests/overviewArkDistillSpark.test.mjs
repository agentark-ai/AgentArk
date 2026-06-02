import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const helperPath = path.join(
  frontendRoot,
  "src",
  "components",
  "overviewArkDistillSpark.ts",
);
const helperSource = readFileSync(helperPath, "utf8");
const transpiled = ts.transpileModule(helperSource, {
  compilerOptions: {
    module: ts.ModuleKind.ES2020,
    target: ts.ScriptTarget.ES2020,
  },
});
const helperModule = await import(
  `data:text/javascript;base64,${Buffer.from(transpiled.outputText).toString("base64")}`
);
const { buildCumulativeSavedTokenSparkValues } = helperModule;

test("builds cumulative ArkDistill saved-token spark values across the analytics window", () => {
  const values = buildCumulativeSavedTokenSparkValues(
    [
      { bucket_start: "2026-05-29T00:00:00+00:00", estimated_saved_tokens: 2746 },
      { bucket_start: "2026-05-30T00:00:00+00:00", estimated_saved_tokens: 1364 },
    ],
    {
      start: "2026-05-28T20:00:00+00:00",
      end: "2026-05-30T20:00:00+00:00",
      bucket: "day",
    },
  );

  assert.deepEqual(values, [0, 2746, 4110]);
});

test("ignores invalid or negative saved-token bucket values when building cumulative values", () => {
  const values = buildCumulativeSavedTokenSparkValues(
    [
      { bucket_start: "2026-05-29T00:00:00+00:00", estimated_saved_tokens: 10 },
      { bucket_start: "2026-05-30T00:00:00+00:00", estimated_saved_tokens: -5 },
      { bucket_start: "2026-05-31T00:00:00+00:00", estimated_saved_tokens: Number.NaN },
    ],
    {
      start: "2026-05-29T00:00:00+00:00",
      end: "2026-05-31T00:00:00+00:00",
      bucket: "day",
    },
  );

  assert.deepEqual(values, [10, 10, 10]);
});
