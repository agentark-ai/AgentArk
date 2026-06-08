import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");
const settingsPageSource = readFileSync(
  path.join(frontendRoot, "src", "components", "pages", "SettingsPageFull.tsx"),
  "utf8",
);

test("identity settings only expose implemented chat identity controls", () => {
  for (const label of ['label="Bot Name"', 'label="Language"', 'label="Tone"']) {
    assert.equal(
      settingsPageSource.includes(label),
      false,
      `the General > Identity settings UI should not render ${label}`,
    );
  }

  assert.equal(
    settingsPageSource.includes('label="Personality"'),
    true,
    'label="Personality" should remain visible in the Identity settings section',
  );
});
