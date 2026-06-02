import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-page-helpers-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/pageHelpers.ts",
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

const { canSaveUserData, memoryRefreshInterval } = await import(
  pathToFileURL(path.join(outDir, "pageHelpers.js")).toString()
);

test("memory refresh continues while consolidation is pending", () => {
  assert.equal(memoryRefreshInterval(false, 0, 8000), false);
  assert.equal(memoryRefreshInterval(false, 2, 8000), 8000);
  assert.equal(memoryRefreshInterval(true, 0, 8000), 8000);
  assert.equal(memoryRefreshInterval(true, 3, 8000), 8000);
});

test("user data save requires a kind, title, and idle mutation", () => {
  assert.equal(canSaveUserData("note", "Travel context", false), true);
  assert.equal(canSaveUserData("", "Travel context", false), false);
  assert.equal(canSaveUserData("note", "   ", false), false);
  assert.equal(canSaveUserData("note", "Travel context", true), false);
});
