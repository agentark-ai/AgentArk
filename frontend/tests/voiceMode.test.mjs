import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-voice-mode-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/chat/voiceMode.ts",
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

const { voiceControlStateFromSession } = await import(
  pathToFileURL(path.join(outDir, "voiceMode.js")).toString()
);

test("shows setup state when the voice runtime is unavailable", () => {
  assert.deepEqual(
    voiceControlStateFromSession({
      status: "unavailable",
      voice_available: false,
      disabled_reason: "voice_not_installed",
    }),
    {
      kind: "setup_needed",
      canStart: false,
      canStop: false,
      label: "Voice setup",
    },
  );
});

test("maps protocol session phases without relying on user wording", () => {
  assert.deepEqual(
    voiceControlStateFromSession({
      voice_available: true,
      session: { id: "voice-1", phase: "listening" },
    }),
    {
      kind: "listening",
      canStart: false,
      canStop: true,
      label: "Listening",
    },
  );

  assert.deepEqual(
    voiceControlStateFromSession({
      voice_available: true,
      session: { id: "voice-1", phase: "speaking" },
    }),
    {
      kind: "speaking",
      canStart: false,
      canStop: true,
      label: "Speaking",
    },
  );
});

test("keeps unknown active protocol phases stoppable", () => {
  assert.deepEqual(
    voiceControlStateFromSession({
      voice_available: true,
      session: { id: "voice-1", phase: "calibrating" },
    }),
    {
      kind: "active",
      canStart: false,
      canStop: true,
      label: "Voice active",
    },
  );
});

test("allows starting when local voice is ready and no session is active", () => {
  assert.deepEqual(
    voiceControlStateFromSession({
      status: "ready",
      voice_available: true,
      session: null,
    }),
    {
      kind: "ready",
      canStart: true,
      canStop: false,
      label: "Start voice",
    },
  );
});
