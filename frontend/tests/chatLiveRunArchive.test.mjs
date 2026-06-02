import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-chat-live-run-archive-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/pages/chatLiveRunArchive.ts",
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

const { buildChatLiveRunArchive } = await import(
  pathToFileURL(path.join(outDir, "chatLiveRunArchive.js")).toString()
);

test("archives live transcript and workspace state under the active conversation", () => {
  const archive = buildChatLiveRunArchive({
    pendingSnapshot: {
      conversationId: "conversation-a",
      message: "build app",
      startedAt: 1000,
      runId: "run-a",
      taskId: "",
      streamingResponse: "old",
      streamingSteps: [{ title: "old step" }],
    },
    taskId: "task-a",
    streamingResponse: "latest assistant progress",
    streamingSteps: [{ title: "writing file" }],
    deployedFiles: [{ name: "app/server.js", content: "server body" }],
    liveFileWrites: {
      "app/server.js": {
        content: "server body",
        line: 10,
        totalLines: 12,
        done: false,
      },
    },
    streamedWorkspaceApp: { id: "app-a", app_dir: "app" },
    codeViewerFileIdx: 2,
    nowMs: 2000,
    maxResponseChars: 1000,
  });

  assert.equal(archive.pendingSnapshot?.conversationId, "conversation-a");
  assert.equal(archive.pendingSnapshot?.taskId, "task-a");
  assert.equal(archive.pendingSnapshot?.streamingResponse, "latest assistant progress");
  assert.deepEqual(archive.pendingSnapshot?.streamingSteps, [{ title: "writing file" }]);
  assert.equal(archive.workspaceSnapshot?.conversationId, "conversation-a");
  assert.equal(archive.workspaceSnapshot?.updatedAt, 2000);
  assert.deepEqual(archive.workspaceSnapshot?.deployedFiles, [
    { name: "app/server.js", content: "server body" },
  ]);
  assert.equal(
    archive.workspaceSnapshot?.liveFileWrites["app/server.js"].content,
    "server body",
  );
  assert.equal(archive.workspaceSnapshot?.streamedWorkspaceApp.id, "app-a");
  assert.equal(archive.workspaceSnapshot?.codeViewerFileIdx, 2);
});

test("does not fabricate an archive without a conversation id", () => {
  const archive = buildChatLiveRunArchive({
    pendingSnapshot: null,
    streamingResponse: "progress",
    streamingSteps: [{ title: "step" }],
    nowMs: 2000,
    maxResponseChars: 1000,
  });

  assert.equal(archive.pendingSnapshot, null);
  assert.equal(archive.workspaceSnapshot, null);
});
