import test from "node:test"
import assert from "node:assert/strict"
import {OrganizationSecretWriteOnly} from "./organization_secret_write_only.js"

const mount = () => {
  const input = {
    value: "request-only-fixture-value",
    defaultValue: "request-only-fixture-value",
    attributes: {value: "request-only-fixture-value"},
    removeAttribute(name) { delete this.attributes[name] },
  }
  const metadata = {name: "credential", modes: ["brokered"]}
  const callbacks = new Map()
  const hook = {
    ...OrganizationSecretWriteOnly,
    el: {querySelector: selector => selector === '[name="secret[value]"]' ? input : metadata},
    handleEvent(event, callback) { callbacks.set(event, callback); return event },
    removeHandleEvent(event) { callbacks.delete(event) },
  }
  hook.mounted()
  return {hook, input, metadata, callbacks}
}

test("an empty server event clears DOM plaintext without changing captured request or metadata", () => {
  const {input, metadata, callbacks} = mount()
  const dispatched = {secret: {...metadata, value: input.value}}
  callbacks.get("organization-secret-clear")({})

  assert.equal(input.value, "")
  assert.equal(input.defaultValue, "")
  assert.equal(input.attributes.value, undefined)
  assert.equal(dispatched.secret.value, "request-only-fixture-value")
  assert.deepEqual(metadata, {name: "credential", modes: ["brokered"]})
})

test("a later failure event clears a newly typed value and remains idempotent", () => {
  const {input, callbacks} = mount()
  const clear = callbacks.get("organization-secret-clear")
  clear({})
  input.value = "second-request-only-value"
  clear({})
  clear({})
  assert.equal(input.value, "")
  assert.equal(input.defaultValue, "")
})

test("destroy clears the original field and removes its event listener", () => {
  const {hook, input, callbacks} = mount()
  hook.destroyed()
  assert.equal(input.value, "")
  assert.equal(input.defaultValue, "")
  assert.equal(callbacks.size, 0)
})

test("a form already removed by revocation can be destroyed safely", () => {
  const {hook, callbacks} = mount()
  hook.el.querySelector = () => null
  callbacks.get("organization-secret-clear")({})
  hook.destroyed()
  assert.equal(callbacks.size, 0)
})
