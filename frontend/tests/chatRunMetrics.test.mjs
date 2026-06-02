import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-chat-run-metrics-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/chatRunMetrics.ts",
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

const { chatRunMetricsFromPayload } = await import(
  pathToFileURL(path.join(outDir, "chatRunMetrics.js")).toString()
);

test("keeps measured TTFT when it precedes total duration", () => {
  assert.equal(
    chatRunMetricsFromPayload({
      input_tokens: 100,
      output_tokens: 20,
      duration_ms: 3500,
      time_to_first_token_ms: 420,
    }).timeToFirstTokenMs,
    420,
  );
});

test("rejects TTFT values that collapse to total duration", () => {
  const metrics = chatRunMetricsFromPayload({
    input_tokens: 100,
    output_tokens: 20,
    duration_ms: 33920,
    time_to_first_token_ms: 33920,
  });

  assert.equal(metrics.durationMs, 33920);
  assert.equal(metrics.timeToFirstTokenMs, undefined);
});

test("rejects TTFT values after total duration", () => {
  assert.equal(
    chatRunMetricsFromPayload({
      input_tokens: 100,
      output_tokens: 20,
      duration_ms: 5000,
      time_to_first_token_ms: 6000,
    }).timeToFirstTokenMs,
    undefined,
  );
});
