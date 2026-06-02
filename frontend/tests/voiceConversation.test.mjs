import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-voice-conversation-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/voice/voiceConversation.ts",
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
  shouldSubmitVoiceTranscript,
  voiceMascotMood,
} = await import(pathToFileURL(path.join(outDir, "voiceConversation.js")).toString());

test("submits only non-empty final transcripts when no turn is already in flight", () => {
  assert.equal(shouldSubmitVoiceTranscript("  open the report  ", false), true);
  assert.equal(shouldSubmitVoiceTranscript("   ", false), false);
  assert.equal(shouldSubmitVoiceTranscript("open the report", true), false);
});

test("maps voice state into mascot moods without user phrase checks", () => {
  assert.equal(voiceMascotMood({ phase: "listening", muted: false }), "listening");
  assert.equal(voiceMascotMood({ phase: "speaking", muted: false }), "speaking");
  assert.equal(voiceMascotMood({ phase: "thinking", muted: false }), "thinking");
  assert.equal(voiceMascotMood({ phase: "listening", muted: true }), "muted");
  assert.equal(voiceMascotMood({ phase: "error", muted: false }), "error");
});
