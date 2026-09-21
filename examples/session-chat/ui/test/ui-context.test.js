import test from "node:test";
import assert from "node:assert/strict";
import { loadUiContext } from "../src/ui-context.js";

const repositoryId = "123e4567-e89b-12d3-a456-426614174000";

function response(body, status = 200) {
  return { ok: status >= 200 && status < 300, status, json: async () => body };
}

test("UI context uses same-origin credentials and returns only the authenticated repository", async () => {
  let request;
  const context = await loadUiContext(async (url, options) => {
    request = { url, options };
    return response({ repository_id: repositoryId });
  });
  assert.deepEqual(context, { repositoryId });
  assert.equal(request.url, "/_heph/ui-context");
  assert.equal(request.options.credentials, "include");
  assert.deepEqual(request.options.headers, { Accept: "application/json" });
});

test("UI context rejects host responses that add fields or select a noncanonical target", async () => {
  await assert.rejects(
    () => loadUiContext(async () => response({ repository_id: repositoryId, actor_id: repositoryId })),
    /invalid repository target/,
  );
  await assert.rejects(
    () => loadUiContext(async () => response({ repository_id: repositoryId.toUpperCase() })),
    /invalid repository target/,
  );
  await assert.rejects(
    () => loadUiContext(async () => response({}, 401)),
    /context is unavailable/,
  );
});
