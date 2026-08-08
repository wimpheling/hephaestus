# First-class SQLite and S3-compatible object primitives

Owner: unassigned

## Outcome

Provide SQLite databases and S3-compatible object buckets as first-class
Hephaestus resources, with declared requirements, exact capability bindings,
auditable use, lifecycle controls, and provider-neutral implementations.

SQLite provides durable structured application state. S3-compatible objects
provide bounded blobs, immutable content bundles, uploads, attachments, and
static web assets. These are product primitives with their own semantics, not
generic host filesystem access or ambient cloud credentials.

```text
release requirement → exact SQLite database or object bucket/prefix binding
                    → immutable workload revision
                    → short-lived runtime authority
                    → controlled SQLite or S3-compatible operation
```

## Locked decisions

| Area | Decision |
| --- | --- |
| Product model | SQLite databases and object buckets are first-class Hephaestus resource kinds, comparable to repositories. Provider traits are implementation details, not the user-facing contract. |
| SQLite ownership | A SQLite database belongs to one project and has explicit lifecycle, backup, restore, migration, and retention records. It is not an arbitrary host path. |
| SQLite concurrency | A writable SQLite binding has one authoritative writer lane at a time. The platform must use a fenced lease or equivalent serialization before mounting or serving a writable database. Read-only bindings are separate, explicit operations. |
| SQLite access | Workloads receive an exact database binding and narrowly scoped operations; they never receive a general host filesystem capability. The initial implementation may use a private mounted database file only behind the platform's writer-lane enforcement. |
| Object model | An object bucket has stable identity. Capabilities bind an exact bucket, optional normalized prefix, and explicit `get`, `put`, `list`, `delete`, and publish/read-manifest operations. Bucket or project membership grants no ambient object authority. |
| Object protocol | The first provider exposes an S3-compatible protocol behind short-lived scoped credentials or a broker. Raw provider credentials and unrestricted bucket access are forbidden. |
| Content publication | Static sites and other public content are published as immutable object manifests. A separate edge/CDN provider may serve a published manifest; publication does not make arbitrary bucket objects public. |
| Gateway use | A synchronous HTTP gateway may use an explicitly bound SQLite database or object bucket. SQLite writes must obey the database writer lane; object operations remain capability checked. |
| Durable objects | Durable-object-style keyed actors are a later composition of released HTTP handlers, key routing, serialized execution, and per-key SQLite state. This task provides prerequisites but does not implement that runtime. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](mvp-03-event-ingress-and-caddy-routing.md)
- [`define-own-the-loop-agent-platform.md`](define-own-the-loop-agent-platform.md)

## Non-goals

This task does not expose arbitrary host files, NFS or POSIX network shares,
general SQL access to platform PostgreSQL, multi-writer SQLite files, arbitrary
S3 provider credentials, a public bucket by default, CDN configuration,
distributed database replication, or durable-object/actor scheduling.

It does not define a generic filesystem abstraction. It does not make an object
bucket a repository, a state volume, or a substitute for capability bindings.

## Implementation checklist

- [ ] **1. Define domain contracts and release requirements**
  - [ ] Add validated SQLite database, object bucket, object prefix, manifest,
    backup, and provider identifiers with deterministic normalized forms.
  - [ ] Extend release configuration with SQLite and object capability slots;
    reject filesystem paths, raw S3 URLs, provider credentials, and tenant
    resource IDs in release source.
  - [ ] Define the SQLite operation vocabulary, including inspect, mount
    read-only, mount read-write, backup, restore, and migrate; define legal
    operation compatibility and deny unsupported combinations.
  - [ ] Define object `get`, `put`, `list`, `delete`, manifest publish, and
    published-manifest read operations, including exact prefix normalization,
    object size limits, content metadata, and conditional-write semantics.
  - [ ] Add parsing, normalization, serialization, duplicate-rejection, and
    illegal-operation tests.

- [ ] **2. Persist and authorize exact resources**
  - [ ] Add authoritative PostgreSQL records for project-owned SQLite databases,
    object buckets, prefixes, manifests, lifecycle state, tombstones, provider
    handles, and audit provenance.
  - [ ] Apply forced RLS and explicit inspect, create, configure, bind, backup,
    restore, publish, and delete permissions.
  - [ ] Require independent authority to grant each database or bucket
    operation to a workload revision; reject cross-project bindings unless a
    specific authorized sharing contract exists.
  - [ ] Add OpenFGA/Mélange relations and real-PostgreSQL tests for tenant
    isolation, revocation, lifecycle CAS, concurrent changes, and historical
    provenance.

- [ ] **3. Implement SQLite lifecycle and writer safety**
  - [ ] Define provision, pause, backup, restore, migrate, delete, and
    recovery transitions, including backup consistency and restore provenance.
  - [ ] Use an authoritative fenced writer lease or equivalent serialized lane
    before writable mount or service; prevent stale workers from writing after
    lease loss.
  - [ ] Define WAL mode, checkpoint, fsync, crash, corruption, database-size,
    and migration failure behavior. Never claim multi-writer safety.
  - [ ] Provide an initial private local provider that mounts a database only
    after authorization and writer-lane acquisition; keep host paths invisible
    to workloads and callers.
  - [ ] Add real-SQLite and real-PostgreSQL failure-injection tests for
    concurrent writers, worker crash, lease expiry, checkpoint failure, backup,
    restore, and stale mount denial.

- [ ] **4. Implement S3-compatible object access**
  - [ ] Define an `ObjectStoreProvider` boundary and implement one S3-compatible
    provider with bucket/prefix scoping, bounded operation requests, and
    provider error normalization.
  - [ ] Issue short-lived scoped credentials or broker each operation; never
    expose unrestricted provider credentials to workloads or queues.
  - [ ] Enforce object key, prefix, byte, content-type, checksum, list-page,
    multipart-upload, and conditional-write limits before provider calls.
  - [ ] Record immutable object-operation audit records without logging object
    bodies or credential material.
  - [ ] Add provider conformance, scope-escape, overwrite race, idempotency,
    credential-expiry, and failure-recovery tests.

- [ ] **5. Publish immutable content bundles**
  - [ ] Define an immutable manifest containing normalized object keys, content
    hashes, metadata, entry point, and source bucket/prefix provenance.
  - [ ] Require an explicit publish operation to make a manifest eligible for
    static-site or other edge delivery; publishing must not broaden bucket
    access or expose unlisted objects.
  - [ ] Define a future content-edge/CDN provider contract, cache identity, and
    invalidation semantics without implementing CDN configuration in this task.
  - [ ] Add tests proving manifest immutability, hash verification, rollback,
    and no public access before publication.

- [ ] **6. Integrate runtime authority and gateway use**
  - [ ] Include exact SQLite/object bindings, operation ceilings, leases, and
    provider scope in immutable authorization snapshots and runtime sessions.
  - [ ] Give a gateway only the selected database/object operations; deny
    ambient repository, state-volume, mailbox, object-bucket, and Caddy
    authority.
  - [ ] Define how synchronous gateway invocation acquires and releases a
    SQLite writer lane, including timeout and client-cancellation behavior.
  - [ ] Add end-to-end scenarios for a gateway reading/writing authorized
    SQLite state and serving or publishing an authorized immutable object
    manifest.

- [ ] **7. Verify and document**
  - [ ] Document the resource models, authority boundaries, writer-lane
    contract, S3-compatible protocol boundary, content-publication semantics,
    backup/recovery, and deliberate durable-object/CDN deferrals.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run real-PostgreSQL, real-SQLite, and S3-provider integration and
    failure-injection scenarios.
  - [ ] Run `git diff --check` and `cargo dev quality` for handoff.

## Completion evidence

Record schema and provider versions, capability and RLS fixtures, SQLite
writer-lease/crash/backup/restore evidence, object-scope and credential-expiry
evidence, immutable-manifest publication evidence, end-to-end gateway fixtures,
test counts, and deliberate follow-up tasks for CDN delivery and durable
objects.
