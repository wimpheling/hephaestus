import {createHash} from "node:crypto"
import {execFileSync} from "node:child_process"
import {readFile} from "node:fs/promises"
import {resolve} from "node:path"
import {fileURLToPath} from "node:url"
import assert from "node:assert/strict"
import {build} from "./build.mjs"

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)))
const packageJson = JSON.parse(await readFile(resolve(packageRoot, "package.json"), "utf8"))
const first = await build()
const second = await build()
const digest = value => createHash("sha256").update(value).digest("hex")
const manifest = JSON.parse(second.manifest)

assert.equal(packageJson.version, "1.0.0")
assert.equal(packageJson.exports["."], "./dist/heph-ui-kit-v1.0.0.css")
assert.equal(packageJson.exports["./manifest"], "./dist/manifest.json")
assert.equal(manifest.version, packageJson.version)
assert.equal(manifest.css_file, "heph-ui-kit-v1.0.0.css")
assert.equal(manifest.css_sha256, digest(second.css))
assert.equal(manifest.token_sha256, digest(second.tokenSource))
assert.equal(first.css, second.css, "the CSS build must be deterministic")
assert.equal(first.manifest, second.manifest, "the manifest must be deterministic")
assert.equal(second.css.includes(second.tokenSource), true, "the exact token source must be included")

for (const forbidden of ["phoenix", "liveview", "heex", "tailwind", "@import"]) {
  assert.equal(second.css.toLowerCase().includes(forbidden), false, `kit must not import ${forbidden}`)
}

for (const className of [
  "heph-ui-kit",
  "heph-ui-container",
  "heph-ui-stack",
  "heph-ui-grid",
  "heph-ui-title",
  "heph-ui-text",
  "heph-ui-panel",
  "heph-ui-button",
  "heph-ui-button--primary",
  "heph-ui-status",
  "heph-ui-label",
  "heph-ui-input",
]) {
  assert.match(second.css, new RegExp(`\\.${className}(?:[^a-zA-Z0-9_-]|$)`))
}

assert.match(second.css, /prefers-reduced-motion/)
assert.match(second.css, /--ink:\s*#171714/)
assert.match(second.css, /\[data-theme="dark"\]/)

const packed = JSON.parse(
  execFileSync("npm", ["pack", "--dry-run", "--json", "--ignore-scripts"], {
    cwd: packageRoot,
    env: {...process.env, npm_config_offline: "true"},
    encoding: "utf8",
  }),
)
const packedFiles = new Set(packed[0].files.map(file => file.path))
for (const expectedFile of [
  "dist/heph-ui-kit-v1.0.0.css",
  "dist/manifest.json",
  "package.json",
  "README.md",
]) {
  assert.equal(packedFiles.has(expectedFile), true, `npm pack must include ${expectedFile}`)
}
console.log("release UI kit tests passed")
