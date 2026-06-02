import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const cssPath = path.join(
  frontendRoot,
  "src",
  "components",
  "pages",
  "chatLanding.css",
);

test("chat landing palette uses native AgentArk colors", () => {
  const css = readFileSync(cssPath, "utf8");
  const bannedNonNativeColors = [
    "#c4b5fd",
    "#a5b4fc",
    "#7dd3fc",
    "#5eead4",
    "139, 92, 246",
    "124, 58, 237",
    "56, 189, 248",
    "45, 212, 191",
    "148, 163, 184",
  ];

  assert.match(css, /120,\s*242,\s*176/);
  assert.match(css, /255,\s*190,\s*99/);
  for (const color of bannedNonNativeColors) {
    assert.equal(css.includes(color), false, `${color} should not be used`);
  }
});
