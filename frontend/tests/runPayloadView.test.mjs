import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const outDir = mkdtempSync(path.join(tmpdir(), "agentark-run-payload-view-"));

execFileSync(
  process.execPath,
  [
    path.join(frontendRoot, "node_modules", "typescript", "bin", "tsc"),
    "src/components/chat/runPayloadView.ts",
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

const { buildRunPayloadView } = await import(
  pathToFileURL(path.join(outDir, "runPayloadView.js")).toString()
);

test("summarizes nested tool input without exposing internal identifiers", () => {
  const view = buildRunPayloadView({
    kind: "scheduled_task",
    action: "notify_user",
    background_session_id: "bg-123",
    content: {
      channel: "telegram",
      task: "Send a notification",
      at: "2026-05-27T23:08:00+05:30",
      api_token: "secret-token",
    },
  });

  assert.ok(view);
  assert.equal(view.kind, "json");
  assert.match(view.preview, /notify user|scheduled task|telegram/i);
  assert.equal(view.body.includes("background_session_id"), true);
  assert.equal(
    view.items.some((item) => item.value.includes("bg-123")),
    false,
  );
  assert.deepEqual(
    view.items.filter((item) => ["Action", "Channel", "Task", "At", "API Token"].includes(item.label)).map((item) => item.label),
    ["Action", "Channel", "Task", "At", "API Token"],
  );
  assert.equal(
    view.items.find((item) => item.label === "API Token")?.value,
    "[redacted]",
  );
});

test("summarizes result arrays and keeps raw JSON available", () => {
  const view = buildRunPayloadView({
    status: "success",
    result: {
      items: [
        { title: "Alpha", url: "https://example.com/a" },
        { title: "Beta", url: "https://example.com/b" },
        { title: "Gamma", url: "https://example.com/c" },
      ],
      total: 3,
    },
    elapsed_ms: 1240,
  });

  assert.ok(view);
  assert.match(view.preview, /success|3 items|elapsed/i);
  assert.equal(view.body.includes('"items"'), true);
  assert.equal(
    view.items.find((item) => item.label === "Elapsed")?.value,
    "1.2s",
  );
  assert.equal(
    view.items.find((item) => item.label === "Result")?.value,
    "Items: 3 items, Total: 3",
  );
});

test("renders plain long text as text output", () => {
  const text = "Line one\nLine two\nLine three with enough detail to be treated as an output payload.";
  const view = buildRunPayloadView(text);

  assert.ok(view);
  assert.equal(view.kind, "text");
  assert.equal(view.badgeLabel, "Output");
  assert.equal(view.body, text);
  assert.equal(view.lineCount, 3);
});
