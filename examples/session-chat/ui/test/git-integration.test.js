import test from "node:test";
import assert from "node:assert/strict";
import * as fs from "node:fs";
import { promises as fsp } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { execFile as execFileCallback } from "node:child_process";
import { promisify } from "node:util";
import git from "isomorphic-git";
import { GitSessionAdapter } from "../src/git-client.js";
import { MAIN_REF, PROTOCOL, VERSION, canonicalJson, makeHumanMessage, pathForRecord } from "../src/protocol.js";

const execFile = promisify(execFileCallback);
const HUMAN = "user:123e4567-e89b-12d3-a456-426614174000";
const RELEASE = "release:reference-chat";
const AGENT = "agent:reference-chat";
const SESSION = "123e4567-e89b-12d3-a456-426614174010";
const CREATED_AT = "2026-09-21T12:00:00Z";

async function gitCommand(directory, args) {
  return execFile("git", ["-C", directory, ...args], { env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null" } });
}

function manifestRecords() {
  return [
    { protocol: PROTOCOL, version: VERSION, record_id: "123e4567-e89b-12d3-a456-426614174011", kind: "session_manifest", actor: { id: RELEASE, role: "release" }, participant_id: RELEASE, created_at: CREATED_AT, data: { session_id: SESSION, release_id: RELEASE, agent_id: AGENT, ref: MAIN_REF } },
    { protocol: PROTOCOL, version: VERSION, record_id: "123e4567-e89b-12d3-a456-426614174012", kind: "participant", actor: { id: RELEASE, role: "release" }, participant_id: RELEASE, created_at: CREATED_AT, data: { role: "release" } },
    { protocol: PROTOCOL, version: VERSION, record_id: "123e4567-e89b-12d3-a456-426614174013", kind: "participant", actor: { id: RELEASE, role: "release" }, participant_id: AGENT, created_at: CREATED_AT, data: { role: "agent" } },
    { protocol: PROTOCOL, version: VERSION, record_id: "123e4567-e89b-12d3-a456-426614174014", kind: "participant", actor: { id: RELEASE, role: "release" }, participant_id: HUMAN, created_at: CREATED_AT, data: { role: "human" } },
  ];
}

async function writeRecords(directory, records) {
  for (const record of records) {
    const path = join(directory, pathForRecord(record));
    await fsp.mkdir(join(path, ".."), { recursive: true });
    await fsp.writeFile(path, `${canonicalJson(record)}\n`);
  }
}

async function commit(directory, message) {
  await gitCommand(directory, ["add", "."]);
  await gitCommand(directory, ["commit", "-m", message]);
}

async function initializeRemote() {
  const root = await mkdtemp(join(tmpdir(), "heph-session-git-"));
  const remote = join(root, "remote.git");
  const source = join(root, "source");
  await execFile("git", ["init", "--bare", "--initial-branch=main", remote]);
  await execFile("git", ["init", "--initial-branch=main", source]);
  await gitCommand(source, ["config", "user.name", "Reference release"]);
  await gitCommand(source, ["config", "user.email", "reference@example.invalid"]);
  await gitCommand(source, ["remote", "add", "origin", remote]);
  await writeRecords(source, manifestRecords());
  await commit(source, "session initialization");
  await gitCommand(source, ["push", "origin", "main"]);
  return { root, remote, source };
}

test("isomorphic-git reads ordinary Git history by first parent and canonical path order", async () => {
  const { root, remote, source } = await initializeRemote();
  const checkout = join(root, "reader");
  await execFile("git", ["clone", "--branch", "main", remote, checkout]);
  const adapter = new GitSessionAdapter({ fs, http: {}, dir: checkout, verifiedActorId: HUMAN.slice(5) });
  const session = await adapter.readSession();
  assert.deepEqual(session.records.map((record) => record.participant_id), [RELEASE, AGENT, RELEASE, HUMAN]);

  const message = makeHumanMessage({ recordId: "123e4567-e89b-12d3-a456-426614174015", actorId: HUMAN, text: "ordered", createdAt: CREATED_AT });
  await writeRecords(source, [message]);
  await commit(source, "human message");
  await gitCommand(source, ["push", "origin", "main"]);
  await gitCommand(checkout, ["fetch", "origin", "main"]);
  await gitCommand(checkout, ["reset", "--hard", "origin/main"]);
  const updated = await adapter.readSession();
  assert.equal(updated.transcript.at(-1).content.text, "ordered");
});

test("release initializes an empty ordinary Git repository with the verified human participant", async () => {
  const root = await mkdtemp(join(tmpdir(), "heph-session-empty-git-"));
  const remote = join(root, "remote.git");
  const checkout = join(root, "initializer");
  await execFile("git", ["init", "--bare", "--initial-branch=main", remote]);
  const cliPushGit = {
    ...git,
    async push({ dir }) {
      await gitCommand(dir, ["push", "origin", "main"]);
      return { ok: true, errors: [] };
    },
  };
  const adapter = new GitSessionAdapter({
    fs,
    http: {},
    dir: checkout,
    repositoryUrl: remote,
    verifiedActorId: HUMAN.slice(5),
    gitApi: cliPushGit,
  });
  const initialized = await adapter.initializeEmptySession({ sessionId: SESSION });
  assert.equal(initialized.transcript.length, 0);
  assert.equal(initialized.records.filter((record) => record.kind === "participant").length, 3);
  assert.equal(initialized.actorId, HUMAN.slice(5));

  const verify = join(root, "verify");
  await execFile("git", ["clone", "--branch", "main", remote, verify], {
    env: { ...process.env, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: "/dev/null" },
  });
  const session = await new GitSessionAdapter({ fs, http: {}, dir: verify, verifiedActorId: HUMAN.slice(5) }).readSession();
  assert.equal(session.records.find((record) => record.kind === "session_manifest").data.session_id, SESSION);
  assert.ok(session.records.some((record) => record.participant_id === HUMAN));
});

test("two ordinary Git writers reconcile a stale push and duplicate record", async () => {
  const { root, remote } = await initializeRemote();
  const writerA = join(root, "writer-a");
  const writerB = join(root, "writer-b");
  await execFile("git", ["clone", "--branch", "main", remote, writerA]);
  await execFile("git", ["clone", "--branch", "main", remote, writerB]);
  for (const directory of [writerA, writerB]) {
    await gitCommand(directory, ["config", "user.name", "Human writer"]);
    await gitCommand(directory, ["config", "user.email", "human@example.invalid"]);
  }
  const first = makeHumanMessage({ recordId: "123e4567-e89b-12d3-a456-426614174016", actorId: HUMAN, text: "first writer", createdAt: CREATED_AT });
  await writeRecords(writerA, [first]);
  await commit(writerA, "first writer");
  await gitCommand(writerA, ["push", "origin", "main"]);

  const second = makeHumanMessage({ recordId: "123e4567-e89b-12d3-a456-426614174017", actorId: HUMAN, text: "second writer", createdAt: CREATED_AT });
  class RetryingAdapter extends GitSessionAdapter {
    async reconnectForRetry() {
      await gitCommand(this.dir, ["fetch", "origin", "main"]);
      await gitCommand(this.dir, ["reset", "--hard", "origin/main"]);
    }
  }
  const cliPushGit = {
    ...git,
    async add(args) {
      return git.add(args);
    },
    async push({ dir }) {
      try {
        await gitCommand(dir, ["push", "origin", "main"]);
        return { ok: true, errors: [] };
      } catch (error) {
        return { ok: false, errors: [error.stderr?.trim() || String(error)] };
      }
    },
  };
  const adapter = new RetryingAdapter({ fs, http: {}, dir: writerB, verifiedActorId: HUMAN.slice(5), gitApi: cliPushGit });
  assert.deepEqual(await adapter.appendHuman(second), second);
  assert.deepEqual(await adapter.appendHuman(second), second);

  const verify = join(root, "verify");
  await execFile("git", ["clone", "--branch", "main", remote, verify]);
  const session = await new GitSessionAdapter({ fs, http: {}, dir: verify, verifiedActorId: HUMAN.slice(5) }).readSession();
  assert.deepEqual(session.transcript.map((record) => record.content.text), ["first writer", "second writer"]);
  assert.equal((await gitCommand(verify, ["rev-list", "--count", "main"])).stdout.trim(), "3");
});
