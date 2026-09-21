export const PROTOCOL = "heph.session-chat";
export const VERSION = 1;
export const MAIN_REF = "refs/heads/main";
export const SESSION_ROOT = ".heph/session/v1";
export const MAX_TEXT_BYTES = 16 * 1024;
export const RELEASE_ID = "release:reference-chat";
export const AGENT_ID = "agent:reference-chat";

export class ProtocolError extends Error {}
export class CompatibilityError extends ProtocolError {}
export class Conflict extends ProtocolError {}

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const ID = /^(release|agent|user):[A-Za-z0-9][A-Za-z0-9._:/-]{0,62}$/;
const TIMESTAMP = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z$/;

function requireObject(value, name) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ProtocolError(`${name} must be an object`);
  }
  return value;
}

function uuid(value, name) {
  if (typeof value !== "string" || !UUID.test(value)) throw new ProtocolError(`${name} must be a canonical UUID`);
  return value;
}

function boundedId(value, name) {
  if (typeof value !== "string" || !ID.test(value)) throw new ProtocolError(`${name} is invalid`);
  if (value.startsWith("user:")) uuid(value.slice(5), name);
  return value;
}

function text(value, name = "text") {
  if (typeof value !== "string" || !value || value.includes("\0") || new TextEncoder().encode(value).byteLength > MAX_TEXT_BYTES) {
    throw new ProtocolError(`${name} is invalid or exceeds ${MAX_TEXT_BYTES} UTF-8 bytes`);
  }
  return value;
}

function exactKeys(value, allowed, required = allowed) {
  const allowedSet = new Set(allowed);
  for (const key of Object.keys(value)) if (!allowedSet.has(key)) throw new ProtocolError(`unknown field ${key}`);
  for (const key of required) if (!(key in value)) throw new ProtocolError(`missing field ${key}`);
}

function content(value) {
  requireObject(value, "content");
  if (value.kind === "text") {
    exactKeys(value, ["kind", "text"]);
    return { kind: "text", text: text(value.text) };
  }
  if (value.kind === "reference") {
    exactKeys(value, ["kind", "content_id", "media_type", "byte_length", "sha256"]);
    uuid(String(value.content_id).replace(/^content:/, ""), "content_id");
    if (!String(value.content_id).startsWith("content:")) throw new ProtocolError("content_id is invalid");
    if (!/^[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]{0,62}\/[A-Za-z0-9][A-Za-z0-9!#$&^_.+-]{0,62}$/.test(value.media_type)) throw new ProtocolError("media_type is invalid");
    if (!Number.isInteger(value.byte_length) || value.byte_length <= 0 || value.byte_length > 1024 * 1024) throw new ProtocolError("byte_length is invalid");
    if (typeof value.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(value.sha256)) throw new ProtocolError("sha256 is invalid");
    return { ...value };
  }
  throw new ProtocolError("unknown content kind");
}

export function parseRecord(value) {
  const record = requireObject(value, "record");
  exactKeys(record, ["protocol", "version", "record_id", "kind", "actor", "participant_id", "created_at", "content", "in_reply_to", "correlation_id", "tombstone_of", "data"], ["protocol", "version", "record_id", "kind", "actor", "participant_id", "created_at"]);
  if (record.protocol !== PROTOCOL || record.version !== VERSION) throw new CompatibilityError("unsupported session-chat protocol");
  uuid(record.record_id, "record_id");
  boundedId(record.participant_id, "participant_id");
  if (typeof record.created_at !== "string" || !TIMESTAMP.test(record.created_at) || Number.isNaN(Date.parse(record.created_at))) throw new ProtocolError("created_at is invalid");
  const actor = requireObject(record.actor, "actor");
  exactKeys(actor, ["id", "role"]);
  boundedId(actor.id, "actor.id");
  if (!["release", "agent", "human"].includes(actor.role)) throw new ProtocolError("actor.role is invalid");
  const prefix = { release: "release:", agent: "agent:", human: "user:" }[actor.role];
  if (!actor.id.startsWith(prefix)) throw new ProtocolError("actor role does not match actor id");
  if (record.data !== undefined) {
    requireObject(record.data, "data");
    for (const [key, item] of Object.entries(record.data)) if (key.length > 64 || typeof item !== "string" || new TextEncoder().encode(item).byteLength > MAX_TEXT_BYTES) throw new ProtocolError("data is invalid");
  }
  if (record.content !== undefined) content(record.content);
  for (const key of ["in_reply_to", "correlation_id", "tombstone_of"]) if (record[key] !== undefined) uuid(record[key], key);
  if (record.kind === "user_message") {
    if (actor.role !== "human" || actor.id !== record.participant_id || !record.content || record.content.kind !== "text") throw new ProtocolError("invalid user message");
    if (record.in_reply_to !== undefined || record.correlation_id !== undefined || record.tombstone_of !== undefined) throw new ProtocolError("user message has response fields");
  } else if (record.kind === "assistant_message") {
    if (actor.role !== "agent" || actor.id !== record.participant_id || !record.content || record.in_reply_to === undefined || record.correlation_id === undefined) throw new ProtocolError("invalid assistant message");
  } else if (record.kind === "tombstone") {
    if (actor.role !== "release" || record.tombstone_of === undefined) throw new ProtocolError("invalid tombstone");
  } else if (record.kind === "participant") {
    const participantData = requireObject(record.data, "participant.data");
    if (actor.role !== "release" || record.content !== undefined || !participantData.role) throw new ProtocolError("invalid participant");
    const participantPrefix = { release: "release:", agent: "agent:", human: "user:" }[participantData.role];
    if (!record.participant_id.startsWith(participantPrefix)) throw new ProtocolError("participant role does not match participant ID");
  } else if (record.kind === "session_manifest") {
    if (actor.role !== "release" || actor.id !== record.participant_id || record.content !== undefined) throw new ProtocolError("invalid manifest");
    const manifestData = requireObject(record.data, "manifest.data");
    for (const key of Object.keys(manifestData)) if (!["session_id", "release_id", "agent_id", "ref", "forked_from_session_id"].includes(key)) throw new ProtocolError(`manifest contains unknown field ${key}`);
    for (const key of ["session_id", "release_id", "agent_id", "ref"]) if (!(key in manifestData)) throw new ProtocolError(`manifest requires ${key}`);
    uuid(manifestData.session_id, "data.session_id");
    if (manifestData.forked_from_session_id !== undefined) uuid(manifestData.forked_from_session_id, "data.forked_from_session_id");
    if (manifestData.ref !== MAIN_REF || manifestData.release_id !== actor.id || !manifestData.agent_id.startsWith("agent:")) throw new ProtocolError("invalid manifest identities");
  } else throw new ProtocolError(`unknown record kind ${record.kind}`);
  return record;
}

function sorted(value) {
  if (Array.isArray(value)) return value.map(sorted);
  if (value && typeof value === "object") return Object.fromEntries(Object.keys(value).sort().map((key) => [key, sorted(value[key])]));
  return value;
}

export function canonicalJson(value) {
  return JSON.stringify(sorted(value));
}

function encoded(value) {
  return encodeURIComponent(value).replace(/[!'()*]/g, (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`);
}

export function pathForRecord(record) {
  const parsed = parseRecord(record);
  if (parsed.kind === "session_manifest") return `${SESSION_ROOT}/manifest.json`;
  if (parsed.kind === "participant") return `${SESSION_ROOT}/participants/${encoded(parsed.participant_id)}.json`;
  if (parsed.kind === "user_message") return `${SESSION_ROOT}/records/human/${encoded(parsed.record_id)}.json`;
  if (parsed.kind === "assistant_message") return `${SESSION_ROOT}/records/agent/${encoded(parsed.participant_id)}/${encoded(parsed.record_id)}.json`;
  if (parsed.kind === "tombstone") return `${SESSION_ROOT}/records/tombstones/${encoded(parsed.tombstone_of)}.json`;
  throw new ProtocolError(`record kind ${parsed.kind} has no browser path`);
}

export function visibleTranscript(records) {
  const removed = new Set(records.filter((record) => record.kind === "tombstone").map((record) => record.tombstone_of));
  return records.filter((record) => ["user_message", "assistant_message"].includes(record.kind) && !removed.has(record.record_id));
}

export function reconcileRecord(records, record) {
  const prior = records.find((candidate) => candidate.record_id === record.record_id);
  if (!prior) return { kind: "append", record };
  if (canonicalJson(prior) === canonicalJson(record)) return { kind: "duplicate", record: prior };
  throw new Conflict(`record ${record.record_id} exists with a different payload`);
}

export function makeHumanMessage({ recordId, actorId, text: messageText, createdAt = new Date().toISOString() }) {
  uuid(recordId, "record_id");
  if (!/^user:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(actorId)) throw new ProtocolError("actorId is not a verified human identity");
  return parseRecord({ protocol: PROTOCOL, version: VERSION, record_id: recordId, kind: "user_message", actor: { id: actorId, role: "human" }, participant_id: actorId, created_at: createdAt, content: { kind: "text", text: messageText } });
}

/**
 * Builds the release-owned baseline for a newly-created repository.
 *
 * The Git principal remains the authenticated human; these protocol actor
 * fields identify the release presentation and are validated by the reader.
 */
export function initialSessionRecords({ sessionId, humanId, createdAt = new Date().toISOString(), releaseId = RELEASE_ID, agentId = AGENT_ID }) {
  uuid(sessionId, "session_id");
  if (!/^user:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(humanId)) throw new ProtocolError("humanId is not a verified human identity");
  const records = [
    { protocol: PROTOCOL, version: VERSION, record_id: crypto.randomUUID(), kind: "session_manifest", actor: { id: releaseId, role: "release" }, participant_id: releaseId, created_at: createdAt, data: { session_id: sessionId, release_id: releaseId, agent_id: agentId, ref: MAIN_REF } },
    { protocol: PROTOCOL, version: VERSION, record_id: crypto.randomUUID(), kind: "participant", actor: { id: releaseId, role: "release" }, participant_id: releaseId, created_at: createdAt, data: { role: "release" } },
    { protocol: PROTOCOL, version: VERSION, record_id: crypto.randomUUID(), kind: "participant", actor: { id: releaseId, role: "release" }, participant_id: agentId, created_at: createdAt, data: { role: "agent" } },
    { protocol: PROTOCOL, version: VERSION, record_id: crypto.randomUUID(), kind: "participant", actor: { id: releaseId, role: "release" }, participant_id: humanId, created_at: createdAt, data: { role: "human" } },
  ];
  return records.map(parseRecord);
}
