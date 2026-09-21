import assert from "node:assert/strict"
import test from "node:test"
import {parseReceiveCommand} from "./session-chat-concurrent-parser.mjs"

const oid = value => value.repeat(40)
const pktLine = command => {
  const length = (command.length + 4).toString(16).padStart(4, "0")
  return Buffer.from(`${length}${command}`, "utf8")
}

test("parses a receive command with NUL capabilities and preserves the ref", () => {
  const parsed = parseReceiveCommand(pktLine(`${oid("a")} ${oid("b")} refs/heads/main\0report-status`))
  assert.deepEqual(parsed, {oldOid: oid("a"), ref: "refs/heads/main"})
})

test("rejects malformed receive commands and invalid packet lengths", () => {
  assert.equal(parseReceiveCommand(pktLine(`${oid("0")} ${oid("b")} refs/heads/main\0`)), undefined)
  assert.equal(parseReceiveCommand(pktLine(`${oid("a")} ${oid("b")} refs/heads/other\0`)), undefined)
  assert.equal(parseReceiveCommand(Buffer.from("0003", "ascii")), undefined)
  assert.equal(parseReceiveCommand(Buffer.from("zzzz", "ascii")), undefined)
})
