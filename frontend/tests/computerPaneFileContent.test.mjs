import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-computer-pane-file-content-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/chat/computerPaneFileContent.ts",
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

const { resolveComputerPaneFileContent } = await import(
  pathToFileURL(path.join(outDir, "computerPaneFileContent.js")).toString()
);

test("uses live file buffer only while the file is actively being written", () => {
  const fullCapturedFile = Array.from({ length: 445 }, (_, index) => {
    return `<p>line ${index + 1}</p>`;
  }).join("\n");
  const staleLiveBuffer = "<!DOCTYPE html>\n<span>partial</span>";

  assert.equal(
    resolveComputerPaneFileContent({
      workspaceContent: fullCapturedFile,
      fallbackContent: "",
      liveWriteContent: staleLiveBuffer,
      isLiveWrite: true,
      liveWriteActive: false,
    }),
    fullCapturedFile,
  );

  assert.equal(
    resolveComputerPaneFileContent({
      workspaceContent: fullCapturedFile,
      fallbackContent: "",
      liveWriteContent: staleLiveBuffer,
      isLiveWrite: true,
      liveWriteActive: true,
    }),
    staleLiveBuffer,
  );
});

test("falls back to live file buffer when no captured content exists", () => {
  assert.equal(
    resolveComputerPaneFileContent({
      workspaceContent: "",
      fallbackContent: "",
      liveWriteContent: "streamed file body",
      isLiveWrite: true,
      liveWriteActive: false,
    }),
    "streamed file body",
  );
});
