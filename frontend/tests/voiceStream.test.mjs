import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-voice-stream-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/voice/voiceStream.ts",
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
  browserVoiceStreamSupport,
  voiceTurnCaptureAction,
  voiceStreamApiPath,
  voiceStreamSocketUrl,
  voiceStreamPhaseFromEvent,
} = await import(pathToFileURL(path.join(outDir, "voiceStream.js")).toString());

test("detects microphone streaming support without browser speech recognition", () => {
  function MediaRecorder() {}
  assert.equal(
    browserVoiceStreamSupport({
      navigator: { mediaDevices: { getUserMedia: async () => ({}) } },
      MediaRecorder,
    }).available,
    true,
  );
  assert.equal(
    browserVoiceStreamSupport({
      navigator: { mediaDevices: { getUserMedia: async () => ({}) } },
    }).available,
    false,
  );
});

test("builds same-origin voice websocket URLs with encoded session ids", () => {
  const url = voiceStreamSocketUrl({
    path: "/voice/sessions/session 1/stream",
    location: {
      protocol: "https:",
      host: "agentark.local",
      origin: "https://agentark.local",
    },
  });

  assert.equal(url, "wss://agentark.local/voice/sessions/session%201/stream");
});

test("builds voice stream API paths with ephemeral stream tokens", () => {
  assert.equal(
    voiceStreamApiPath("session 1", "token+/="),
    "/voice/sessions/session%201/stream?stream_token=token%2B%2F%3D",
  );
});

test("uses explicit sequential turn capture instead of continuous multi-turn VAD", () => {
  assert.equal(
    voiceTurnCaptureAction({
      sessionActive: true,
      recording: false,
      busy: false,
      requested: "start",
    }),
    "start_turn_capture",
  );
  assert.equal(
    voiceTurnCaptureAction({
      sessionActive: true,
      recording: true,
      busy: false,
      requested: "finish",
    }),
    "finish_turn_capture",
  );
  assert.equal(
    voiceTurnCaptureAction({
      sessionActive: true,
      recording: false,
      busy: true,
      requested: "start",
    }),
    null,
  );
});

test("maps structured voice stream events into interactive phases", () => {
  assert.equal(voiceStreamPhaseFromEvent({ type: "session.ready" }), "listening");
  assert.equal(voiceStreamPhaseFromEvent({ type: "agent.thinking" }), "thinking");
  assert.equal(voiceStreamPhaseFromEvent({ type: "tts.audio" }), "speaking");
  assert.equal(voiceStreamPhaseFromEvent({ type: "session.listening" }), "listening");
  assert.equal(voiceStreamPhaseFromEvent({ type: "error" }), "error");
  assert.equal(voiceStreamPhaseFromEvent({ type: "unknown" }), null);
});
