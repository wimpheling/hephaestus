import test from "node:test"
import assert from "node:assert/strict"
import {InstalledUiNavigation, safeBootstrapUrl} from "./installed_ui_navigation.js"

const fragment = "a".repeat(43)
const generationHost = "g-0123456789abcdef0123456789abcdef.ui.example.com"
const bootstrapUrl = port =>
  `https://${generationHost}${port ? `:${port}` : ""}/_heph/bootstrap?heph_theme=light#${fragment}`

class ClassList {
  constructor(...values) {
    this.values = new Set(values)
  }

  add(value) {
    this.values.add(value)
  }

  remove(value) {
    this.values.delete(value)
  }

  contains(value) {
    return this.values.has(value)
  }
}

class Element {
  constructor(dataset = {}) {
    this.dataset = dataset
    this.classList = new ClassList("hidden")
    this.children = new Map()
    this.listeners = new Map()
    this.contentWindow = {messages: [], postMessage: (...message) => this.contentWindow.messages.push(message)}
    this.hidden = false
    this.src = ""
    this.textContent = ""
  }

  querySelector(selector) {
    return this.children.get(selector) || null
  }

  querySelectorAll(selector) {
    const children = this.children.get(selector)
    return Array.isArray(children) ? children : []
  }

  getAttribute(attribute) {
    return this.attributes?.[attribute] || null
  }

  focus() {
    this.focused = true
  }

  addEventListener(event, callback) {
    this.listeners.set(event, callback)
  }

  removeEventListener(event) {
    this.listeners.delete(event)
  }

  removeAttribute(attribute) {
    if (attribute === "src") this.src = ""
  }
}

const timers = []
const messages = new Set()
const documentElement = {dataset: {theme: "dark"}}

globalThis.document = {documentElement}
globalThis.window = {
  location: {replace: value => (globalThis.window.replaced = value)},
  addEventListener: (event, callback) => event === "message" && messages.add(callback),
  removeEventListener: (event, callback) => event === "message" && messages.delete(callback),
  setTimeout: (callback, delay) => {
    const timer = {callback, delay, canceled: false}
    timers.push(timer)
    return timer
  },
  clearTimeout: timer => (timer.canceled = true),
}
globalThis.MutationObserver = class {
  constructor(callback) {
    this.callback = callback
    globalThis.lastObserver = this
  }

  observe() {}

  disconnect() {}
}

const sendMessage = (source, origin, data) => {
  for (const callback of messages) callback({source, origin, data})
}

const makeHook = (port = "443") => {
  const root = new Element()
  const el = new Element({uiNamespace: "ui.example.com", uiPort: port})
  el.parentElement = root
  const frame = new Element()
  const status = new Element()
  const terminal = new Element()
  const close = new Element()
  const launchButton = new Element()
  launchButton.attributes = {"phx-value-id": "installation-1"}
  root.children.set("[data-ui-terminal-status]", terminal)
  root.children.set("[data-ui-installation] button", [launchButton])
  el.children.set("[data-ui-frame]", frame)
  el.children.set("[data-ui-status]", status)
  el.children.set("[data-ui-close]", close)

  const hook = {
    ...InstalledUiNavigation,
    el,
    callbacks: new Map(),
    handleEvent(event, callback) {
      this.callbacks.set(event, callback)
    },
  }

  hook.mounted()
  return {hook, el, root, frame, status, terminal, close, launchButton}
}

test("validates the configured public port, including normalized default HTTPS", () => {
  assert.ok(safeBootstrapUrl(bootstrapUrl(), "ui.example.com", 443))
  assert.ok(safeBootstrapUrl(bootstrapUrl(443), "ui.example.com", 443))
  assert.ok(safeBootstrapUrl(bootstrapUrl(8443), "ui.example.com", 8443))
  assert.equal(safeBootstrapUrl(bootstrapUrl(), "ui.example.com", 8443), null)
  assert.equal(safeBootstrapUrl(bootstrapUrl(8443), "ui.example.com", 443), null)
})

test("launches with the host theme and sends only exact-origin theme messages", () => {
  const {hook, frame, status} = makeHook()
  hook.launch({url: bootstrapUrl(), mode: "iframe"})

  assert.match(frame.src, /heph_theme=dark/)
  assert.equal(status.textContent, "Loading")

  sendMessage({}, "https://g-0123456789abcdef0123456789abcdef.ui.example.com", {type: "heph-ui-ready"})
  assert.equal(status.textContent, "Loading")
  sendMessage(frame.contentWindow, "https://other.example.com", {type: "heph-ui-ready"})
  assert.equal(status.textContent, "Loading")

  sendMessage(frame.contentWindow, "https://g-0123456789abcdef0123456789abcdef.ui.example.com", {type: "heph-ui-ready"})
  assert.equal(status.textContent, "Ready")
  assert.deepEqual(frame.contentWindow.messages.at(-1), [
    {type: "heph-ui-theme", theme: "dark"},
    "https://g-0123456789abcdef0123456789abcdef.ui.example.com",
  ])

  documentElement.dataset.theme = "light"
  lastObserver.callback()
  assert.deepEqual(frame.contentWindow.messages.at(-1), [
    {type: "heph-ui-theme", theme: "light"},
    "https://g-0123456789abcdef0123456789abcdef.ui.example.com",
  ])
})

test("closes terminal child states and exposes bounded timeout status outside the frame", () => {
  const first = makeHook()
  first.hook.launch({url: bootstrapUrl(), mode: "iframe"})
  sendMessage(first.frame.contentWindow, "https://g-0123456789abcdef0123456789abcdef.ui.example.com", {
    type: "heph-ui-state",
    state: "revoked",
  })
  assert.equal(first.el.classList.contains("hidden"), true)
  assert.equal(first.frame.src, "")
  assert.equal(first.terminal.classList.contains("hidden"), false)
  assert.match(first.terminal.textContent, /no longer available/)

  const second = makeHook()
  second.hook.launch({url: bootstrapUrl(), mode: "iframe"})
  timers.at(-1).callback()
  assert.equal(second.el.classList.contains("hidden"), true)
  assert.match(second.terminal.textContent, /did not respond/)
})

test("handles an explicit close through the sibling status root and restores launch focus", () => {
  const {hook, close, launchButton} = makeHook()
  const pushed = []
  hook.pushEvent = (...event) => pushed.push(event)
  hook.launch({url: bootstrapUrl(), mode: "iframe", installation_id: "installation-1"})

  close.listeners.get("click")()

  assert.deepEqual(pushed, [["close-installed-ui", {}]])
  assert.equal(launchButton.focused, true)
})
