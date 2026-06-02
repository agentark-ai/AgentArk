import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const memoryPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "MemoryPage.tsx"),
  "utf8",
);

test("MemoryPage exposes learned memory categories counted by the backend", () => {
  const backendLearnedCategories = [
    "profile_fact",
    "assistant_preference",
    "work_preference",
    "project_domain_memory",
    "ephemeral_context",
    "other",
  ];

  for (const category of backendLearnedCategories) {
    assert.match(
      memoryPageSource,
      new RegExp(`category=${category}`),
      `missing learned memory category query for ${category}`,
    );
  }
});
