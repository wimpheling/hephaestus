import test from "node:test";
import assert from "node:assert/strict";
import { browserHttp } from "../src/git-client.js";

test("browser Git transport includes browser credentials without exposing cookie or bearer headers", async () => {
  const previousFetch = globalThis.fetch;
  let seen;
  globalThis.fetch = async (_url, options) => {
    seen = options;
    return new Response(new Uint8Array(), { status: 200, headers: { "Heph-Git-Actor-Id": "123e4567-e89b-12d3-a456-426614174000" } });
  };
  try {
    let actor;
    const client = browserHttp((response) => { actor = response.headers["heph-git-actor-id"]; });
    const response = await client.request({ url: "https://example.test/_heph/git/repo/info/refs?service=git-upload-pack", method: "GET", headers: { Accept: "application/x-git-upload-pack-advertisement" } });
    assert.equal(response.statusCode, 200);
    assert.equal(actor, "123e4567-e89b-12d3-a456-426614174000");
    assert.equal(seen.credentials, "include");
    assert.equal(seen.headers.Authorization, undefined);
    assert.equal(seen.headers.Cookie, undefined);
  } finally {
    globalThis.fetch = previousFetch;
  }
});
