import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { URL, URLSearchParams } from "node:url";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

const SCRIPT = readFileSync(
  fileURLToPath(new URL("../src/ui_bootstrap.js", import.meta.url)),
  "utf8",
);
const SECRET = "a".repeat(43);
const PLATFORM_ORIGIN = "https://app.example";
const UI_ORIGIN = "https://g-0123456789abcdef0123456789abcdef.ui.app.example";

async function runScript({
  hash = `#${SECRET}`,
  search = "",
  response = { route: "/docs/index.html", theme_origin: PLATFORM_ORIGIN },
  platformOrigin = PLATFORM_ORIGIN,
} = {}) {
  const events = [];
  const navigation = [];
  const document = {
    body: { textContent: "" },
    querySelector(selector) {
      assert.equal(selector, 'meta[name="heph-platform-origin"]');
      return { content: platformOrigin };
    },
  };
  const location = {
    hash,
    search,
    pathname: "/",
    origin: UI_ORIGIN,
    replace(value) {
      navigation.push(value);
    },
  };
  const history = {
    replaceState(_state, _title, value) {
      events.push({ type: "history", value });
    },
  };
  const fetch = (url, init) => {
    events.push({ type: "fetch", url, init });
    return Promise.resolve({
      ok: true,
      json: async () => response,
    });
  };
  vm.runInNewContext(
    SCRIPT,
    {
      URL,
      URLSearchParams,
      document,
      fetch,
      history,
      location,
      Promise,
    },
    { filename: "ui_bootstrap.js" },
  );
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
  return { document, events, navigation };
}

test("clears fragment before fetch and forwards only canonical theme", async () => {
  const result = await runScript({
    search: "?heph_theme=dark",
    response: {
      route: "/docs/index.html?heph_theme=dark",
      theme_origin: PLATFORM_ORIGIN,
    },
  });
  assert.deepEqual(result.events.map(({ type }) => type), ["history", "fetch"]);
  assert.equal(result.events[0].value, "/?heph_theme=dark");
  assert.equal(result.events[1].url, "/_heph/bootstrap?heph_theme=dark");
  assert.equal(result.events[1].init.body, SECRET);
  assert.equal(result.events[1].init.credentials, "same-origin");
  assert.deepEqual(result.navigation, [
    "/docs/index.html?heph_theme=dark&heph_theme_origin=https%3A%2F%2Fapp.example",
  ]);
  assert.equal(result.document.body.textContent, "");
});

test("invalid fragment and duplicate or unknown theme do not POST", async (t) => {
  await t.test("invalid fragment", async () => {
    const result = await runScript({ hash: "#short" });
    assert.equal(result.events.length, 1);
    assert.equal(result.events[0].type, "history");
    assert.deepEqual(result.navigation, []);
    assert.equal(result.document.body.textContent.includes("short"), false);
  });
  await t.test("duplicate theme", async () => {
    const result = await runScript({ search: "?heph_theme=light&heph_theme=dark" });
    assert.deepEqual(result.events.map(({ type }) => type), ["history"]);
    assert.deepEqual(result.navigation, []);
  });
  await t.test("unknown theme value", async () => {
    const result = await runScript({ search: "?heph_theme=blue" });
    assert.deepEqual(result.events.map(({ type }) => type), ["history"]);
    assert.deepEqual(result.navigation, []);
  });
  await t.test("unknown query key", async () => {
    const result = await runScript({ search: "?unknown=drop" });
    assert.deepEqual(result.events.map(({ type }) => type), ["history"]);
    assert.deepEqual(result.navigation, []);
  });
});

test("rejects unsafe response path and foreign platform origin", async (t) => {
  await t.test("absolute response path", async () => {
    const result = await runScript({
      response: { route: "https://evil.example/steal", theme_origin: PLATFORM_ORIGIN },
    });
    assert.deepEqual(result.navigation, []);
    assert.equal(result.document.body.textContent.includes(SECRET), false);
  });
  await t.test("foreign theme origin", async () => {
    const result = await runScript({
      response: { route: "/docs/index.html", theme_origin: "https://evil.example" },
    });
    assert.deepEqual(result.navigation, []);
    assert.equal(result.document.body.textContent.includes(SECRET), false);
  });
  await t.test("encoded or traversal path", async () => {
    const result = await runScript({
      response: { route: "/docs/%2e%2e/steal", theme_origin: PLATFORM_ORIGIN },
    });
    assert.deepEqual(result.navigation, []);
  });
  await t.test("terminal dot segments", async () => {
    for (const route of ["/docs/.", "/docs/..", "/docs/.?heph_theme=dark"]) {
      const result = await runScript({
        response: { route, theme_origin: PLATFORM_ORIGIN },
      });
      assert.deepEqual(result.navigation, []);
    }
  });
});
