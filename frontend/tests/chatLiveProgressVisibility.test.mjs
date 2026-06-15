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

function functionSource(source, functionName) {
  const start = source.indexOf(`function ${functionName}`);
  assert.notEqual(start, -1, `${functionName} should exist`);
  const braceStart = source.indexOf("{", start);
  assert.notEqual(braceStart, -1, `${functionName} should have a body`);
  let depth = 0;
  for (let index = braceStart; index < source.length; index += 1) {
    const ch = source[index];
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }
  }
  assert.fail(`${functionName} body should close`);
}

test("live chat transcript action groups are expanded during streaming", () => {
  assert.equal(
    /const expanded\s*=\s*\(isLiveTranscript && hasRunning\)\s*\|\|\s*expandedTranscriptActions\.has\(groupId\);/.test(
      chatPageSource,
    ),
    true,
    "live transcript action groups should expand while running and auto-collapse once settled",
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

test("chat thread does not render internal reasoning as assistant reply text", () => {
  assert.equal(
    chatPageSource.includes("const visibleLiveModelEmit"),
    false,
    "chat should not derive a live assistant emit from internal reasoning",
  );
  assertSourceIncludes(
    chatPageSource,
    "visibleStreamingResponse.trim()",
    "streaming assistant bubble should render from final/user-visible response tokens",
  );
  assert.equal(
    chatPageSource.includes("visibleLiveModelEmit.trim()"),
    false,
    "reasoning-derived live model emits should not keep a chat reply bubble open",
  );
  assert.equal(
    chatPageSource.includes("deferredLiveModelEmitText"),
    false,
    "streaming markdown should not fall back to internal reasoning text",
  );
});

test("live chat transcript keeps model prose progress alongside tool actions", () => {
  assertSourceIncludes(
    chatPageSource,
    'kind === "model_prose"',
    "chat transcript builder should recognize model_prose progress events",
  );
  assert.equal(
    /item\.kind === "action"[\s\S]{0,120}item\.kind === "prose"/.test(
      chatPageSource,
    ),
    true,
    "live transcript should keep model prose rows instead of filtering to actions only",
  );
  assert.equal(
    /pushPendingProse\(\);\s*const finalItems = runLooksComplete\(\)/.test(
      chatPageSource,
    ),
    true,
    "pending model prose should flush even before any tool action arrives",
  );
});

test("live chat transcript preserves model prose across active step limiting", () => {
  assertSourceIncludes(
    chatPageSource,
    "function isTranscriptPreservedActivityStep",
    "step limiting should have a shared predicate for chat-visible prose rows",
  );
  assertSourceIncludes(
    chatPageSource,
    "isTranscriptPreservedActivityStep(step)",
    "live and pending step limiters should preserve model_prose rows instead of keeping only tail tool rows",
  );
  assertSourceIncludes(
    chatPageSource,
    "const liveTranscriptSourceSteps = useMemo(",
    "live transcript should derive from a merged source that cannot drop prose when tool rows arrive",
  );
  assertSourceIncludes(
    chatPageSource,
    "mergeActivityStepSourcesForLiveTranscript(",
    "live transcript source should merge preserved prose rows with active tool steps",
  );
});

test("live chat transcript only preserves public model_prose as chat prose", () => {
  const modelNarrationSource = functionSource(
    chatPageSource,
    "modelNarrationTextFromActivityStep",
  );
  assert.equal(
    modelNarrationSource.includes("modelProseTextFromActivityStep(step)"),
    true,
    "public model_prose events should remain chat-visible",
  );
  assert.equal(
    modelNarrationSource.includes("isMainChatReasoningStep"),
    false,
    "internal reasoning_delta events should not become chat transcript prose",
  );
  assert.equal(
    modelNarrationSource.includes("agentLoopProgressPhaseFromStep"),
    false,
    "agent loop model-call progress is console/run-status metadata, not chat prose",
  );
});

test("live chat transcript item cap preserves model prose rows", () => {
  assertSourceIncludes(
    chatPageSource,
    "preserveProse?: boolean",
    "transcript limiting should support preserving model prose independently of the action cap",
  );
  assertSourceIncludes(
    chatPageSource,
    "limitTranscriptItemsForDisplay(finalItems, maxItems, options)",
    "transcript builder should not use a raw tail slice that drops earlier model emits",
  );
  assertSourceIncludes(
    chatPageSource,
    "preserveProse: true",
    "live transcript should keep model emits visible while actions continue streaming",
  );
});

test("main chat transcript shows thinking content without JSON audit blocks", () => {
  const preservedSource = functionSource(
    chatPageSource,
    "isTranscriptPreservedActivityStep",
  );
  assert.equal(
    preservedSource.includes("isMainChatReasoningStep(step)"),
    true,
    "reasoning_delta steps should survive transcript limiting so Thinking content can render",
  );
  const internalReasoningIndex = chatPageSource.indexOf(
    "const internalReasoningText = modelInternalReasoningTextFromActivityStep(step);",
  );
  assert.notEqual(
    internalReasoningIndex,
    -1,
    "transcript builder should inspect internal reasoning",
  );
  const proseIndex = chatPageSource.indexOf(
    "const proseText = modelNarrationTextFromActivityStep(step);",
    internalReasoningIndex,
  );
  assert.notEqual(
    proseIndex,
    -1,
    "transcript builder should handle public prose after internal reasoning",
  );
  const reasoningGuardSource = chatPageSource.slice(
    internalReasoningIndex,
    proseIndex,
  );
  assertSourceIncludes(
    reasoningGuardSource,
    "if (internalReasoningText)",
    "transcript builder should guard internal reasoning",
  );
  assertSourceIncludes(
    reasoningGuardSource,
    "appendThinkingRow(",
    "transcript builder should surface internal reasoning as a Thinking content row",
  );
  assertSourceIncludes(
    reasoningGuardSource,
    "continue;",
    "transcript builder should stop internal reasoning before tool-action handling",
  );
  assert.equal(
    reasoningGuardSource.includes("appendReasoningDetail"),
    false,
    "internal reasoning guard should not append a main-chat row",
  );
  assert.equal(
    chatPageSource.includes("fullReasoningDetail"),
    false,
    "chat transcript should not use the removed fullReasoningDetail variable",
  );
  assert.equal(
    chatPageSource.includes('kind: "reasoning";'),
    false,
    "main chat transcript items should not include a stale reasoning row variant",
  );
  assertSourceIncludes(
    chatPageSource,
    'kind: "thinking";',
    "main chat transcript items should include a thinking row variant",
  );
  assertSourceIncludes(
    chatPageSource,
    "details: ChatTranscriptActionDetail[];",
    "Thinking rows should carry plain text detail entries",
  );
  assertSourceIncludes(
    chatPageSource,
    'title: "Thinking"',
    "main chat transcript should label internal reasoning activity as Thinking",
  );
  assert.equal(
    chatPageSource.includes("renderReasoning"),
    false,
    "main chat transcript renderer should not keep a stale reasoning preview branch",
  );
  assertSourceIncludes(
    chatPageSource,
    "renderThinking",
    "main chat transcript renderer should keep a Thinking content branch",
  );
  const renderThinkingIndex = chatPageSource.indexOf("const renderThinking = (");
  assert.notEqual(
    renderThinkingIndex,
    -1,
    "Thinking renderer should exist",
  );
  const renderThinkingEnd = chatPageSource.indexOf(
    "// Walk items, grouping consecutive action items into runs.",
    renderThinkingIndex,
  );
  assert.notEqual(
    renderThinkingEnd,
    -1,
    "Thinking renderer source should be bounded before item grouping",
  );
  const renderThinkingSource = chatPageSource.slice(
    renderThinkingIndex,
    renderThinkingEnd,
  );
  assert.equal(
    renderThinkingSource.includes("item.detail ?"),
    false,
    "collapsed Thinking row should not show an inline reasoning preview",
  );
  assert.equal(
    renderThinkingSource.includes("chat-transcript-action-separator"),
    false,
    "collapsed Thinking row should not render the preview separator",
  );
  assertSourceIncludes(
    chatPageSource,
    "renderTranscriptActionDetail(item.id, entry, entryIdx,",
    "Thinking rows should render their text details",
  );
  assertSourceIncludes(
    chatPageSource,
    "showAudit: false",
    "Thinking rows should suppress JSON command/output audit blocks",
  );
  assert.equal(
    /item\.kind === "action"[\s\S]{0,180}item\.kind === "prose"[\s\S]{0,180}item\.kind === "reasoning"/.test(
      chatPageSource,
    ),
    false,
    "live transcript filter should not keep reasoning-detail rows in the main chat",
  );
  assert.equal(
    /item\.kind === "action"[\s\S]{0,180}item\.kind === "prose"[\s\S]{0,180}item\.kind === "thinking"/.test(
      chatPageSource,
    ),
    true,
    "live transcript filter should keep Thinking rows in the main chat",
  );
  assert.equal(
    chatPageSource.includes('item.kind === "action" || item.kind === "reasoning"'),
    false,
    "completed run transcript filter should not keep reasoning-detail rows in the main chat",
  );
  assertSourceIncludes(
    chatPageSource,
    'item.kind === "action" || item.kind === "thinking"',
    "completed run transcript filter should keep Thinking rows in the main chat",
  );
});
