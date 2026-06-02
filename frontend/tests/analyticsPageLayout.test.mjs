import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const analyticsPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "AnalyticsPage.tsx"),
  "utf8",
);

test("analytics hero summary cards use a single five-card desktop row", () => {
  assert.match(
    analyticsPageSource,
    /lg:\s*"repeat\(5,\s*minmax\(0,\s*1fr\)\)"/,
    "hero summary grid should fit its five cards in one desktop row",
  );
});
