import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-companion-pairing-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/companionPairing.ts",
    "--ignoreConfig",
    "--target",
    "ES2020",
    "--module",
    "ES2020",
    "--moduleResolution",
    "Bundler",
    "--outDir",
    outDir,
    "--skipLibCheck"
  ],
  { cwd: frontendRoot, stdio: "inherit" }
);
writeFileSync(path.join(outDir, "package.json"), JSON.stringify({ type: "module" }));

const { sessionNeedsPairingPoll } = await import(
  pathToFileURL(path.join(outDir, "companionPairing.js")).toString()
);

test("keeps polling after a pairing session is approved", () => {
  assert.equal(
    sessionNeedsPairingPoll(
      {
        pairing_sessions: [
          {
            id: "pairing-1",
            status: "approved"
          }
        ]
      },
      "pairing-1"
    ),
    true
  );
});

test("stops polling after a pairing session is completed", () => {
  assert.equal(
    sessionNeedsPairingPoll(
      {
        pairing_sessions: [
          {
            id: "pairing-1",
            status: "completed"
          }
        ]
      },
      "pairing-1"
    ),
    false
  );
});
