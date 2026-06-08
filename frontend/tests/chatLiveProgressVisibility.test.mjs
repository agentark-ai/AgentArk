import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const chatPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "ChatPage.tsx"),
  "utf8",
);
const computerPaneSource = readFileSync(
  path.join(frontendRoot, "src", "components", "chat", "ComputerPane.tsx"),
  "utf8",
);

function assertSourceIncludes(source, needle, message) {
  assert.equal(source.includes(needle), true, message);
}

test("live chat transcript action groups are expanded during streaming", () => {
  assertSourceIncludes(
    chatPageSource,
    "const expanded = isLiveTranscript || expandedTranscriptActions.has(groupId);",
    "live transcript action groups should expand without requiring a click",
  );
  assertSourceIncludes(
    chatPageSource,
    '<Collapse in={expanded} timeout="auto" unmountOnExit>',
    "action group body should use the computed expanded state",
  );
});

test("computer console working view receives live assistant token preview", () => {
  assert.equal(
    /<WorkingView[\s\S]{0,420}tokenPreview=\{tokenPreview\}/.test(computerPaneSource),
    true,
    "working view should receive the existing token preview stream",
  );
});

test("chat thread renders live model emits before final answer tokens arrive", () => {
  assertSourceIncludes(
    chatPageSource,
    "const visibleLiveModelEmit = useMemo(",
    "chat should derive a visible live model emit from the reasoning stream",
  );
  assertSourceIncludes(
    chatPageSource,
    'reasoningStream?.content || ""',
    "live model emits should use the actual streamed model reasoning content",
  );
  assertSourceIncludes(
    chatPageSource,
    "visibleStreamingResponse.trim() || visibleLiveModelEmit.trim()",
    "streaming assistant bubble should render for final tokens or model emits",
  );
  assertSourceIncludes(
    chatPageSource,
    "const visibleStreamingMarkdownText = visibleStreamingResponse.trim()",
    "streaming markdown should choose final answer text before model emits",
  );
});

test("live chat transcript keeps model prose progress alongside tool actions", () => {
  assertSourceIncludes(
    chatPageSource,
    'kind === "model_prose"',
    "chat transcript builder should recognize model_prose progress events",
  );
  assertSourceIncludes(
    chatPageSource,
    'item.kind === "action" || item.kind === "prose"',
    "live transcript should keep model prose rows instead of filtering to actions only",
  );
});
