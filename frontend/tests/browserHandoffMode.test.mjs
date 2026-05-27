import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-browser-handoff-mode-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/browserHandoffMode.ts",
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

const { isProfileBrowserSession } = await import(
  pathToFileURL(path.join(outDir, "browserHandoffMode.js")).toString()
);

test("detects saved-profile browser sessions from structured session fields", () => {
  assert.equal(
    isProfileBrowserSession({
      profile_id: "profile-1",
      conversation_id: null,
    }),
    true,
  );
  assert.equal(
    isProfileBrowserSession({
      profile_id: "profile-1",
      conversation_id: "conversation-1",
    }),
    false,
  );
  assert.equal(
    isProfileBrowserSession({
      profile_id: null,
      conversation_id: null,
    }),
    false,
  );
});
