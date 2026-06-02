import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-voice-history-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/voice/voiceHistory.ts",
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
  VOICE_CONVERSATION_STORAGE_KEY,
  loadPersistedVoiceConversationId,
  persistVoiceConversationId,
  voiceTurnsFromConversationMessages,
} = await import(pathToFileURL(path.join(outDir, "voiceHistory.js")).toString());

test("maps saved conversation messages into voice turns", () => {
  assert.deepEqual(
    voiceTurnsFromConversationMessages({
      messages: [
        { id: "m1", role: "user", content: " Hello ", timestamp: "t1" },
        { id: "m2", role: "assistant", content: " Hi ", timestamp: "t2" },
        { id: "m3", role: "tool", content: "hidden", timestamp: "t3" },
        { id: "m4", role: "assistant", content: "   ", timestamp: "t4" },
      ],
    }),
    [
      { id: "m1", role: "user", content: "Hello", timestamp: "t1" },
      { id: "m2", role: "assistant", content: "Hi", timestamp: "t2" },
    ],
  );
});

test("persists the last voice conversation id without requiring a live socket", () => {
  const values = new Map();
  const storage = {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
    removeItem: (key) => values.delete(key),
  };

  persistVoiceConversationId(storage, "  convo-1  ");
  assert.equal(values.get(VOICE_CONVERSATION_STORAGE_KEY), "convo-1");
  assert.equal(loadPersistedVoiceConversationId(storage), "convo-1");

  persistVoiceConversationId(storage, "");
  assert.equal(loadPersistedVoiceConversationId(storage), null);
});
