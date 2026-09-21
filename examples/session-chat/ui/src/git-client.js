import git from "isomorphic-git";
import http from "isomorphic-git/http/web";
import LightningFS from "@isomorphic-git/lightning-fs";
import {
  MAIN_REF,
  SESSION_ROOT,
  canonicalJson,
  parseRecord,
  pathForRecord,
  reconcileRecord,
  visibleTranscript,
  Conflict,
  ProtocolError,
  initialSessionRecords,
} from "./protocol.js";

const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
const ACTOR_HEADER = "heph-git-actor-id";

function canonicalUuid(value) {
  if (typeof value !== "string" || !UUID.test(value)) throw new Error("the Git route requires a canonical repository UUID");
  return value;
}

function bytes(value) {
  return new TextEncoder().encode(value);
}

const UTF8 = new TextEncoder();
function compareUtf8(left, right) {
  const a = UTF8.encode(left);
  const b = UTF8.encode(right);
  const length = Math.min(a.length, b.length);
  for (let index = 0; index < length; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return a.length - b.length;
}

async function writeText(fs, path, value) {
  const parent = path.slice(0, path.lastIndexOf("/"));
  const fileApi = fs.promises ?? fs;
  if (parent) await ensureDirectory(fileApi, parent);
  await fileApi.writeFile(path, bytes(`${value}\n`));
}

async function ensureDirectory(fileApi, directory) {
  let current = "";
  for (const component of directory.split("/").filter(Boolean)) {
    current += `/${component}`;
    try {
      await fileApi.stat(current);
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
      await fileApi.mkdir(current);
    }
  }
}

function sameOriginUrl(path) {
  const url = new URL(path, window.location.origin);
  if (url.origin !== window.location.origin) throw new Error("session Git must use the installed UI origin");
  return url;
}

export function browserHttp(onResponse, signalProvider = () => undefined) {
  return {
    async request(request) {
      // The browser supplies the HttpOnly child cookie and Origin. This adapter
      // never reads document.cookie and never creates an Authorization header.
      const response = await http.request({
        ...request,
        fetchOptions: {
          ...(request.fetchOptions ?? {}),
          credentials: "include",
          signal: request.signal ?? signalProvider() ?? request.fetchOptions?.signal,
        },
      });
      onResponse?.(response);
      return response;
    },
  };
}

export class GitSessionAdapter {
  constructor({ fs, http: httpClient, dir = "/session", repositoryUrl, humanDisplayName = "Session user", verifiedActorId, gitApi = git }) {
    this.fs = fs;
    this.http = httpClient;
    this.repositoryUrl = repositoryUrl;
    this.git = gitApi;
    this.dir = dir;
    this.humanDisplayName = humanDisplayName;
    this.verifiedActorId = verifiedActorId;
  }

  async readSession() {
    const records = [];
    const seen = new Map();
    const commits = [];
    for (let oid = await this.git.resolveRef({ fs: this.fs, dir: this.dir, ref: MAIN_REF }); oid;) {
      const commit = await this.git.readCommit({ fs: this.fs, dir: this.dir, oid });
      commits.push(commit);
      oid = commit.commit.parent?.[0];
    }
    commits.reverse();
    let parentTree = new Map();
    let manifest;
    const participants = new Map();
    for (const commit of commits) {
      const currentTree = await this.readProtocolTree(commit.commit.tree);
      const changedPaths = [...new Set([...parentTree.keys(), ...currentTree.keys()])]
        .filter((path) => parentTree.get(path)?.oid !== currentTree.get(path)?.oid)
        .sort(compareUtf8);
      for (const path of changedPaths) {
        if (path.includes("/context/") || path.includes("/content/") || path.endsWith("/context.json")) continue;
        const current = currentTree.get(path);
        if (!current) throw new ProtocolError(`protocol path deleted from history: ${path}`);
        const parsed = parseRecord(JSON.parse(current.text));
        if (current.text !== `${canonicalJson(parsed)}\n`) throw new ProtocolError(`record is not canonical JSON: ${path}`);
        if (pathForRecord(parsed) !== path) throw new ProtocolError(`record is stored at the wrong path: ${path}`);
        const old = parentTree.get(path);
        if (old && parsed.kind !== "session_manifest") throw new ProtocolError(`immutable record path changed: ${path}`);
        const prior = seen.get(parsed.record_id);
        if (prior && canonicalJson(prior) !== canonicalJson(parsed)) throw new ProtocolError(`history contains conflicting record ID: ${parsed.record_id}`);
        if (!prior || parsed.kind === "session_manifest") {
          seen.set(parsed.record_id, parsed);
          records.push(parsed);
        }
        if (parsed.kind === "session_manifest") manifest = parsed;
        if (parsed.kind === "participant") participants.set(parsed.participant_id, parsed.data.role);
      }
      parentTree = currentTree;
    }
    if (!manifest) throw new ProtocolError("session history has no manifest");
    if (participants.get(manifest.actor.id) !== "release" || participants.get(manifest.data.agent_id) !== "agent") {
      throw new ProtocolError("manifest identities are not registered participants");
    }
    for (const record of records) {
      if (["session_manifest", "participant"].includes(record.kind)) continue;
      if (participants.get(record.actor.id) !== record.actor.role) throw new ProtocolError(`record actor is not a registered participant: ${record.actor.id}`);
    }
    return { records, transcript: visibleTranscript(records), actorId: this.verifiedActorId };
  }

  async readProtocolTree(oid) {
    const files = new Map();
    const visit = async (treeOid, prefix) => {
      const tree = await this.git.readTree({ fs: this.fs, dir: this.dir, oid: treeOid });
      for (const entry of tree.tree) {
        const path = prefix ? `${prefix}/${entry.path}` : entry.path;
        if (entry.type === "tree") await visit(entry.oid, path);
        else if (entry.type === "blob" && path.startsWith(`${SESSION_ROOT}/`) && path.endsWith(".json")) {
          const blob = await this.git.readBlob({ fs: this.fs, dir: this.dir, oid: entry.oid });
          files.set(path, { oid: entry.oid, text: new TextDecoder().decode(blob.blob) });
        }
      }
    };
    await visit(oid, "");
    return files;
  }

  async appendHuman(record, attempts = 3) {
    const parsed = parseRecord(record);
    if (parsed.actor.id !== `user:${this.verifiedActorId}`) throw new Error("human record actor is not the verified Git actor");
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      const current = await this.readSession();
      const outcome = reconcileRecord(current.records, parsed);
      if (outcome.kind === "duplicate") return outcome.record;
      const path = pathForRecord(parsed);
      await writeText(this.fs, `${this.dir}/${path}`, canonicalJson(parsed));
      await this.git.add({ fs: this.fs, dir: this.dir, filepath: path });
      await this.git.commit({ fs: this.fs, dir: this.dir, ref: MAIN_REF, message: `session human message ${parsed.record_id}`, author: { name: this.humanDisplayName, email: "human@session.invalid" }, committer: { name: this.humanDisplayName, email: "human@session.invalid" }, disallowEmpty: true });
      try {
        const result = await this.git.push({ fs: this.fs, http: this.http, dir: this.dir, remote: "origin", ref: "main", headers: {} });
        if (result?.ok === false || result?.errors?.length) {
          throw new Error(result.errors?.join("; ") || "Git push was rejected");
        }
        return parsed;
      } catch (error) {
        if (attempt === attempts - 1) throw error;
        await this.reconnectForRetry(error);
      }
    }
    throw new Conflict("Git append retry budget exhausted");
  }

  async reconnectForRetry(error) {
    throw new Conflict(`Git append requires a fresh checkout after a rejected push: ${error instanceof Error ? error.message : String(error)}`);
  }

  async initializeEmptySession({ sessionId = crypto.randomUUID(), humanId = `user:${this.verifiedActorId}` } = {}) {
    if (!this.verifiedActorId || humanId !== `user:${this.verifiedActorId}`) throw new Error("empty session initialization requires the verified Git actor");
    await this.git.init({ fs: this.fs, dir: this.dir, defaultBranch: "main" });
    await this.git.addRemote({ fs: this.fs, dir: this.dir, remote: "origin", url: this.repositoryUrl });
    for (const record of initialSessionRecords({ sessionId, humanId })) {
      const path = pathForRecord(record);
      await writeText(this.fs, `${this.dir}/${path}`, canonicalJson(record));
      await this.git.add({ fs: this.fs, dir: this.dir, filepath: path });
    }
    await this.git.commit({
      fs: this.fs,
      dir: this.dir,
      ref: MAIN_REF,
      message: "session initialization",
      author: { name: this.humanDisplayName, email: "human@session.invalid" },
      committer: { name: this.humanDisplayName, email: "human@session.invalid" },
      disallowEmpty: true,
    });
    const result = await this.git.push({ fs: this.fs, http: this.http, dir: this.dir, remote: "origin", ref: "main", headers: {} });
    if (result?.ok === false || result?.errors?.length) throw new Conflict(result.errors?.join("; ") || "empty session initialization was rejected");
    return this.readSession();
  }
}

export class SessionGitClient extends GitSessionAdapter {
  constructor({ repositoryId, repositoryUrl, storageName = `heph-session-${repositoryId}`, humanDisplayName = "Session user" }) {
    const canonicalRepositoryId = canonicalUuid(repositoryId);
    const url = sameOriginUrl(repositoryUrl ?? `/_heph/git/${canonicalRepositoryId}`);
    if (url.pathname !== `/\u005fheph/git/${canonicalRepositoryId}` && url.pathname !== `/\u005fheph/git/${canonicalRepositoryId}/`) throw new Error("repository URL is outside the reserved Git route");
    let verifiedActorId;
    let activeSignal;
    const clientHttp = browserHttp((response) => {
      const actor = response.headers?.[ACTOR_HEADER];
      if (actor !== undefined) {
        if (!UUID.test(actor)) throw new Error("server returned an invalid verified Git actor");
        verifiedActorId = actor;
      }
    }, () => activeSignal);
    const fs = new LightningFS(storageName).promises;
    super({ fs, http: clientHttp, repositoryUrl: url.href.replace(/\/$/, ""), humanDisplayName });
    this.repositoryId = canonicalRepositoryId;
    this._browserVerifiedActorId = () => verifiedActorId;
    this._setFetchSignal = (signal) => { activeSignal = signal; };
  }

  async connect() {
    // A fresh checkout makes reconnect reconcile the complete reachable history.
    this.dir = `/session-${crypto.randomUUID()}`;
    const info = await this.git.getRemoteInfo({ fs: this.fs, http: this.http, url: this.repositoryUrl, headers: {} });
    this.verifiedActorId = this._browserVerifiedActorId();
    if (!this.verifiedActorId) throw new Error("Git discovery did not return a verified human actor");
    const heads = info.refs?.heads ?? {};
    if (Object.keys(heads).length === 0) return this.initializeEmptySession();
    if (!heads.main) throw new ProtocolError("session repository has no main branch");
    await this.git.clone({ fs: this.fs, http: this.http, dir: this.dir, url: this.repositoryUrl, ref: "main", singleBranch: true, headers: {} });
    return this.readSession();
  }

  async reconnect() {
    return this.connect();
  }

  async reconnectForRetry() {
    await this.reconnect();
  }

  async fetch({ signal } = {}) {
    this._setFetchSignal?.(signal);
    try {
      await this.git.fetch({ fs: this.fs, http: this.http, dir: this.dir, remote: "origin", ref: "main", singleBranch: true, headers: {}, signal });
      await this.git.fastForward({ fs: this.fs, http: this.http, dir: this.dir, ref: "main", remote: "origin", singleBranch: true, headers: {} });
      return this.readSession();
    } finally {
      this._setFetchSignal?.(undefined);
    }
  }

}

export { bytes };
