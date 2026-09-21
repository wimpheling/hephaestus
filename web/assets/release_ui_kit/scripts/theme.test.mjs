import assert from "node:assert/strict"
import {readFile} from "node:fs/promises"
import test from "node:test"
import vm from "node:vm"
import {resolve} from "node:path"
import {fileURLToPath} from "node:url"

const packageRoot = resolve(fileURLToPath(new URL("..", import.meta.url)))
const source = await readFile(resolve(packageRoot, "src/theme.js"), "utf8")

function run({search = "", iframe = true, readyState = "complete"} = {}) {
  const listeners = new Map()
  const messages = []
  const parent = {postMessage: (data, origin) => messages.push({data, origin})}
  const document = {
    readyState,
    documentElement: {dataset: {}},
    addEventListener: (name, callback) => listeners.set(name, callback),
  }
  const window = {location: {search}, addEventListener: (name, callback) => listeners.set(name, callback)}
  window.parent = iframe ? parent : window
  const context = vm.createContext({URLSearchParams, URL, window, document})
  vm.runInContext(source, context, {filename: "theme.js"})
  return {
    document,
    messages,
    dispatchMessage(data, {source = parent, origin = "https://platform.example"} = {}) {
      listeners.get("message")?.({data, source, origin})
    },
    finishDom() {
      listeners.get("DOMContentLoaded")?.()
    },
  }
}

test("initializes an explicit theme and leaves the OS fallback unset", () => {
  assert.equal(run({search: "?heph_theme=dark"}).document.documentElement.dataset.theme, "dark")
  assert.equal(run({search: ""}).document.documentElement.dataset.theme, undefined)
  assert.equal(run({search: "?heph_theme=blue"}).document.documentElement.dataset.theme, undefined)
})

test("sends ready only from an iframe after DOM ready to the exact origin", () => {
  const loading = run({search: "?heph_theme_origin=https%3A%2F%2Fplatform.example", readyState: "loading"})
  assert.deepEqual(loading.messages, [])
  loading.finishDom()
  assert.equal(loading.messages.length, 1)
  assert.equal(loading.messages[0].origin, "https://platform.example")
  assert.equal(loading.messages[0].data.type, "heph-ui-ready")

  const fullPage = run({search: "?heph_theme_origin=https%3A%2F%2Fplatform.example", iframe: false})
  assert.deepEqual(fullPage.messages, [])
})

test("accepts only closed theme messages from the exact parent and origin", () => {
  const ui = run({search: "?heph_theme_origin=https%3A%2F%2Fplatform.example"})
  ui.dispatchMessage({type: "heph-ui-theme", theme: "dark"})
  assert.equal(ui.document.documentElement.dataset.theme, "dark")
  ui.dispatchMessage({type: "heph-ui-theme", theme: "light", extra: true})
  ui.dispatchMessage({type: "heph-ui-theme", theme: "light"}, {origin: "https://foreign.example"})
  ui.dispatchMessage({type: "heph-ui-theme", theme: "light"}, {source: {}})
  ui.dispatchMessage({type: "heph-ui-theme", theme: "blue"})
  assert.equal(ui.document.documentElement.dataset.theme, "dark")
})

test("rejects duplicate or malformed origin and theme query values", () => {
  const duplicateTheme = run({search: "?heph_theme=light&heph_theme=dark"})
  assert.equal(duplicateTheme.document.documentElement.dataset.theme, undefined)
  const duplicateOrigin = run({search: "?heph_theme_origin=https%3A%2F%2Fplatform.example&heph_theme_origin=https%3A%2F%2Fplatform.example"})
  assert.deepEqual(duplicateOrigin.messages, [])
  const pathOrigin = run({search: "?heph_theme_origin=https%3A%2F%2Fplatform.example%2Fui"})
  assert.deepEqual(pathOrigin.messages, [])
})
