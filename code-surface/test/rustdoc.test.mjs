import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { generateRustApi, parseRustdocAllHtml, parseRustdocModulesHtml } from "../scripts/rustdoc.mjs";

const testDirectory = dirname(fileURLToPath(import.meta.url));
const fixture = readFileSync(resolve(testDirectory, "fixtures/rustdoc-all.html"), "utf8");

test("rustdoc all page inventory is public, linked, and deterministic", () => {
  const items = parseRustdocAllHtml(fixture, { baseHref: "target/doc/demo/" });
  assert.deepEqual(items, [
    { kind: "constant", name: "VERSION", href: "target/doc/demo/constant.VERSION.html" },
    { kind: "enum", name: "Mode", href: "target/doc/demo/enum.Mode.html" },
    { kind: "function", name: "parse", href: "target/doc/demo/fn.parse.html" },
    { kind: "module", name: "nested", href: "target/doc/demo/nested/index.html" },
    { kind: "struct", name: "Config", href: "target/doc/demo/struct.Config.html" },
    { kind: "struct", name: "nested::Item", href: "target/doc/demo/nested/struct.Item.html" },
    { kind: "trait", name: "Render", href: "target/doc/demo/trait.Render.html" },
    { kind: "type", name: "Result", href: "target/doc/demo/type.Result.html" },
  ]);
});

test("rustdoc crate index contributes public top-level modules", () => {
  const modules = parseRustdocModulesHtml('<h2 id="modules">Modules</h2><dl class="item-table"><dt><a class="mod" href="ui/index.html">ui<wbr></a></dt></dl>', { baseHref: "target/doc/demo/" });
  assert.deepEqual(modules, [{ kind: "module", name: "ui", href: "target/doc/demo/ui/index.html" }]);
});

test("rustdoc inventory reports crates without library targets clearly", () => {
  const [binary, library] = generateRustApi([
    { name: "binary", targets: [{ name: "binary", kind: ["bin"] }] },
    { name: "library", targets: [{ name: "library", kind: ["lib"] }] },
  ], { docRoot: resolve(testDirectory, "fixtures/does-not-exist") });
  assert.deepEqual(binary, { status: "no-library-target", items: [] });
  assert.deepEqual(library, { status: "unavailable", items: [] });
});
