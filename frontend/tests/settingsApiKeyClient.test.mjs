import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { createServer } from "vite";

const testDir = path.dirname(fileURLToPath(import.meta.url));
const frontendRoot = path.resolve(testDir, "..");

let server;
let clientModule;

test.before(async () => {
  server = await createServer({
    root: frontendRoot,
    logLevel: "silent",
    server: { middlewareMode: true },
  });
  clientModule = await server.ssrLoadModule("/src/api/client.ts");
});

test.after(async () => {
  await server?.close();
});

test("normalizes API key GET metadata without retaining a full key", () => {
  assert.equal(
    typeof clientModule.normalizeSettingsApiKeyMetadata,
    "function",
  );

  const metadata = clientModule.normalizeSettingsApiKeyMetadata({
    key: "ak_live_full_secret",
    masked: "ak_live_...1234",
    issued_at_unix: 100,
    expires_at_unix: 200,
    remaining_seconds: 100,
    rotated: true,
  });

  assert.equal(metadata.key, undefined);
  assert.equal(metadata.masked, "ak_live_...1234");
  assert.equal(metadata.issued_at_unix, 100);
  assert.equal(metadata.rotated, true);
});

test("reveals the full API key with a master-password POST", async () => {
  const { api } = clientModule;
  assert.equal(typeof api.revealSettingsApiKey, "function");

  const calls = [];
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return new Response(
      JSON.stringify({ key: "ak_live_full_secret", masked: "ak_live_...1234" }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  };

  try {
    const response = await api.revealSettingsApiKey("master-pass");
    assert.equal(response.key, "ak_live_full_secret");
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "/settings/api-key/reveal");
    assert.equal(calls[0].init.method, "POST");
    assert.deepEqual(JSON.parse(calls[0].init.body), {
      master_password: "master-pass",
    });
  } finally {
    globalThis.fetch = previousFetch;
  }
});

test("regenerates the full API key with a master-password POST", async () => {
  const { api } = clientModule;
  assert.equal(typeof api.regenerateSettingsApiKey, "function");

  const calls = [];
  const previousFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    calls.push({ url: String(url), init });
    return new Response(
      JSON.stringify({ key: "ak_live_new_secret", masked: "ak_live_...5678" }),
      { status: 200, headers: { "Content-Type": "application/json" } },
    );
  };

  try {
    const response = await api.regenerateSettingsApiKey("master-pass");
    assert.equal(response.key, "ak_live_new_secret");
    assert.equal(calls.length, 1);
    assert.equal(calls[0].url, "/settings/api-key/regenerate");
    assert.equal(calls[0].init.method, "POST");
    assert.deepEqual(JSON.parse(calls[0].init.body), {
      master_password: "master-pass",
    });
  } finally {
    globalThis.fetch = previousFetch;
  }
});
