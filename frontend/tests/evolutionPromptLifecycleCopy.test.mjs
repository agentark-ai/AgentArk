import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const evolutionPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "EvolutionPage.tsx"),
  "utf8",
);

test("prompt lifecycle rejection copy explains rejected candidates without phrase-specific branches", () => {
  assert.match(
    evolutionPageSource,
    /function promptLifecycleReasonCopy\(/,
    "prompt lifecycle reason copy should be centralized",
  );
  assert.doesNotMatch(
    evolutionPageSource,
    /statistical confidence is not high enough/i,
    "copy logic should not depend on one exact gate reason",
  );
  assert.match(
    evolutionPageSource,
    /ArkEvolve did not have enough evidence to safely replace the stable prompt\./,
    "rejected candidates should explain the outcome plainly",
  );
  assert.match(
    evolutionPageSource,
    /Nothing was rolled out\. The current stable prompt is still serving users while this candidate stays available for review\./,
    "copy should explain that stable stayed live",
  );
  assert.match(
    evolutionPageSource,
    /This failed test is not deployable, but the idea can be tested again after new examples are collected\./,
    "copy should distinguish a retryable idea from an undeployable failed result",
  );
  assert.match(
    evolutionPageSource,
    /Wait for more examples, then rerun the background test if the idea still looks useful\./,
    "copy should give a concrete next step",
  );
  assert.match(
    evolutionPageSource,
    /Why it stayed out of production/,
    "the reason panel should use a user-facing title",
  );
  assert.match(
    evolutionPageSource,
    /Background test finished; stable stayed live because the candidate was not proven better\./,
    "row helper should avoid gate jargon",
  );
  assert.match(
    evolutionPageSource,
    /lifecycleStatus === "candidate_rejected"[\s\S]*samples < required/,
    "rejected candidates with enough fresh samples should re-enter the active retry list",
  );
});

test("prompt lifecycle copy distinguishes optimizer failures from candidate rejection gates", () => {
  assert.match(
    evolutionPageSource,
    /function promptLifecycleReasonCopy\(\s*formattedReason: string,\s*lifecycleStatus: string,\s*jobStatus: string,/,
    "copy helper should receive the GEPA job status",
  );
  assert.match(
    evolutionPageSource,
    /sampleCount = 0,\s*requiredSamples = 0,/,
    "copy helper should receive lifecycle sample progress",
  );
  assert.match(
    evolutionPageSource,
    /Background optimization did not finish\./,
    "failed GEPA work should not be explained as a candidate evidence gate",
  );
  assert.match(
    evolutionPageSource,
    /freshSampleGap > 0[\s\S]*Collect \$\{freshSampleGap\.toLocaleString\(\)\} fresh prompt telemetry sample/,
    "optimizer failures should point at the fresh-sample gate when retry is blocked by telemetry",
  );
  assert.match(
    evolutionPageSource,
    /lifecycleStatus === "candidate_rejected"[\s\S]*optimizerFailure/,
    "optimizer failures should be handled before candidate-rejected copy",
  );
  assert.match(
    evolutionPageSource,
    /str\(lifecycle\.job_status, ""\)/,
    "the lifecycle job status should be passed into the reason copy helper",
  );
  assert.match(
    evolutionPageSource,
    /promptLifecycleReasonCopy\([\s\S]*lifecycleSamples,[\s\S]*lifecycleRequiredSamples,/,
    "the lifecycle sample gate should be passed into the reason copy helper",
  );
});
