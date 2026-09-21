import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import {
  CompatibilityError,
  Conflict,
  canonicalJson,
  makeHumanMessage,
  parseRecord,
  pathForRecord,
  reconcileRecord,
  visibleTranscript,
} from "../src/protocol.js";

const pythonFixture = JSON.parse(await readFile(fileURLToPath(new URL("./fixtures/python-session.json", import.meta.url)), "utf8"));

const actor = "user:123e4567-e89b-12d3-a456-426614174000";
const record = makeHumanMessage({ recordId: "123e4567-e89b-12d3-a456-426614174001", actorId: actor, text: "hello", createdAt: "2026-09-21T12:00:00Z" });

test("protocol fixture has canonical record shape and locked human path", () => {
  assert.equal(pathForRecord(record), ".heph/session/v1/records/human/123e4567-e89b-12d3-a456-426614174001.json");
  assert.equal(canonicalJson({ z: 1, a: { y: 2, x: 3 } }), '{"a":{"x":3,"y":2},"z":1}');
  assert.deepEqual(visibleTranscript([record]), [record]);
});

test("reader rejects future protocol versions and malformed human identity", () => {
  assert.throws(() => parseRecord({ ...record, version: 2 }), CompatibilityError);
  assert.throws(() => makeHumanMessage({ recordId: record.record_id, actorId: "user:not-a-uuid", text: "hello", createdAt: record.created_at }), /actorId/);
});

test("stale writer reconciliation preserves duplicate no-op and rejects ID conflicts", () => {
  assert.equal(reconcileRecord([], record).kind, "append");
  assert.equal(reconcileRecord([record], record).kind, "duplicate");
  const changed = { ...record, content: { kind: "text", text: "different" } };
  assert.throws(() => reconcileRecord([record], changed), Conflict);
});

test("Python reference fixture is accepted with the same paths and transcript", () => {
  const records = pythonFixture.records.map(parseRecord);
  assert.deepEqual(records.map(pathForRecord), pythonFixture.paths);
  assert.deepEqual(visibleTranscript(records).map((record) => record.record_id), pythonFixture.transcript_ids);
  for (const record of records) assert.equal(JSON.stringify(JSON.parse(canonicalJson(record))), canonicalJson(record));
});

test("content references remain valid protocol records for the renderer", () => {
  const reference = parseRecord({
    protocol: "heph.session-chat",
    version: 1,
    record_id: "123e4567-e89b-12d3-a456-426614174018",
    kind: "assistant_message",
    actor: { id: "agent:reference-chat", role: "agent" },
    participant_id: "agent:reference-chat",
    created_at: "2026-09-21T12:00:00Z",
    content: { kind: "reference", content_id: "content:123e4567-e89b-12d3-a456-426614174019", media_type: "text/plain", byte_length: 12, sha256: "a".repeat(64) },
    in_reply_to: "123e4567-e89b-12d3-a456-426614174015",
    correlation_id: "123e4567-e89b-12d3-a456-426614174017",
  });
  assert.equal(reference.content.kind, "reference");
});
