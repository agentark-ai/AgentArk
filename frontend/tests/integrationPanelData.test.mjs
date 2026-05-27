import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-integration-panel-data-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/integrationPanelData.ts",
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

const { asRecord } = await import(
  pathToFileURL(path.join(outDir, "integrationPanelData.js")).toString()
);

test("returns a stable empty record for non-record values", () => {
  const first = asRecord(undefined);
  const second = asRecord(null);
  const third = asRecord([]);

  assert.equal(first, second);
  assert.equal(second, third);
  assert.deepEqual(first, {});
});

test("returns existing records without cloning", () => {
  const value = { settings: { slack: { policy: "open" } } };

  assert.equal(asRecord(value), value);
});
