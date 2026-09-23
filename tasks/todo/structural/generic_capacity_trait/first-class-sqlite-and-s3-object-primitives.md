# First-class private volumes, SQLite, and S3-compatible object primitives

Owner: unassigned

## Outcome

Provide bounded private volumes, SQLite databases, and S3-compatible object
buckets as distinct first-class Hephaestus resources, with declared
requirements, exact capability bindings, auditable use, lifecycle controls,
and provider-neutral implementations.

Private volumes provide bounded filesystem storage only through an explicit
volume binding and controlled attachment. SQLite provides durable structured
application state with database-specific operations and lifecycle. S3-compatible
objects provide blobs, immutable content bundles, uploads, attachments, and
static web assets. These are separate product primitives, not generic host
filesystem access or ambient cloud credentials.

```text
release requirements → exact private-volume, SQLite-database, or
                       object-bucket/prefix bindings
                    → immutable workload revision
                    → short-lived runtime authority
                    → controlled volume attachment or typed SQLite/S3 operation
```

## Locked decisions

| Area | Decision |
| --- | --- |
| Product model | Private volumes, SQLite databases, and object buckets are distinct first-class Hephaestus resource kinds, comparable to repositories. Each kind has a provider-neutral contract and a type-specific provider boundary; do not force volume, SQL, and object operations into one universal provider interface. Provider choices are implementation details, not the user-facing contract. |
| Resource ownership | Every private volume, SQLite database, and object bucket has exactly one owning project. Legacy instance-state volumes inherit the project of their owning instance during migration. A resource binding requires explicit authority for that exact resource and operation; project membership alone grants no use or lifecycle authority. Cross-project binding is denied unless a separately authorized sharing contract is added. |
| Typed resource slots | A release may declare zero or more uniquely named slots for each resource kind. Each immutable instance revision binds every required slot to exactly one authorized same-project resource and may leave an optional slot unbound; undeclared slots and multiple assignments to one slot are rejected. Slot declarations and bindings are part of the immutable revision. There is no implicit instance-wide volume fallback. |
| Private volumes | A private volume is an explicitly declared, capacity-bounded storage resource. Volume-slot attachment and guest mount paths are explicit and bounded. |
| Volume access and lifecycle | The declared access mode distinguishes read-only from read-write use. A volume has at most one writable attachment at a time. A new writable attachment is denied until the previous writer is proven detached or fenced by the runtime/provider; lease expiry alone is not proof. If detach or fencing cannot be confirmed, retain the lease/recovery state and fail closed. Volumes have explicit provision, inspect, attach/detach, backup, restore, retention, and delete lifecycle records. |
| Volume providers | Extend the existing instance-state `VolumeStore` and related metadata contracts; this is not a new contract from zero. Public resource, release, authorization-snapshot, and runtime-facing contracts expose stable typed IDs and bounded claims, never `host_id`, `host_path`, filesystem paths, provider credentials, or provider-specific configuration. A provider adapter may retain local paths and opaque provider handles internally. Preserve a local provider as the initial implementation. |
| SQLite ownership | A SQLite database belongs to one project and has explicit lifecycle, backup, restore, migration, and retention records. It is not an arbitrary host path. |
| SQLite provider and access | SQLite has a database-specific provider contract and exact database binding with narrowly scoped operations. Its storage may use a private volume internally, but the database identity, authorization, migrations, backup, and lifecycle remain distinct from that volume. Workloads never receive a general host filesystem capability. The provider API and read-only access mechanism are implementation decisions recorded before implementation. |
| SQLite concurrency | A writable SQLite binding has one authoritative writer lane at a time. The platform must use a fenced lease or equivalent serialization before granting writable access, and must prove prior writer detach/fencing before granting a new writer. Do not assume read-only concurrency or snapshot semantics until they are specified. |
| Object model | An object bucket has stable identity. Capabilities bind an exact bucket, optional normalized prefix, and explicit `get`, `put`, `list`, `delete`, and publish/read-manifest operations. Bucket or project membership grants no ambient object authority. |
| Object provider and protocol | Object storage has its own provider contract. The first provider implements a documented, bounded S3-compatible operation subset behind short-lived scoped credentials or a broker. Raw provider credentials and unrestricted bucket access are forbidden. Select the credential mechanism, consistency/versioning contract, multipart behavior, and numeric limits before implementation. |
| Content publication | Static sites and other public content are published as immutable object manifests. A separate edge/CDN provider may serve a published manifest; publication does not make arbitrary bucket objects public. |
| Gateway use | A synchronous HTTP gateway may use an explicitly bound SQLite database or object bucket. SQLite writes must obey the database writer lane; object operations remain capability checked. |
| Durable objects | Durable-object-style keyed actors are a later composition of released HTTP handlers, key routing, serialized execution, and per-key SQLite state. This task provides prerequisites but does not implement that runtime. |

## Dependencies

- [`mvp-01-agent-principals-capabilities-and-runtime-authority.md`](../../../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md)
- [`mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md`](../../../done/mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md)
- [`mvp-03-event-ingress-and-caddy-routing.md`](../../../done/mvp-03-event-ingress-and-caddy-routing.md)
- [`define-own-the-loop-agent-platform.md`](../../distribution/define-own-the-loop-agent-platform.md)
- Coordinate package placement and architecture metadata with the sibling [workspace topology task](../code_architecture/clarify-workspace-crate-topology-and-extension-boundaries.md). Keep its 85-existing-package plus five-facade baseline intact; account for any packages added by this task as separate additions during the later inventory reconciliation.

## Non-goals

This task does not expose arbitrary host files, unbounded filesystem access,
NFS or POSIX network shares, general SQL access to platform PostgreSQL,
multi-writer SQLite files, arbitrary S3 provider credentials, a public bucket by
default, CDN configuration, distributed database replication, or
durable-object/actor scheduling.

Private volumes are explicit, bounded resources with exact capability bindings;
they do not provide a generic host-filesystem abstraction. An object bucket is
not a filesystem volume or repository, and none of these resources substitutes
for capability bindings.

## Implementation checklist

- [ ] **1. Define typed resource contracts and release requirements**
  - [ ] Add validated private-volume, volume-slot, SQLite database, object
    bucket, object prefix, manifest, backup, and provider identifiers with
    deterministic normalized forms.
  - [ ] Extend release configuration with zero-or-more named private-volume
    slots and typed SQLite/object capability slots; reject host paths, raw S3
    URLs, provider credentials, and tenant resource IDs in release source.
  - [ ] Define the SQLite operation vocabulary, including inspect, authorized
    data access, backup, restore, and migrate; define legal operation
    compatibility and deny unsupported combinations. Decide whether data access
    uses a database protocol, a controlled mount, or another provider API, and
    specify read-only consistency/concurrency semantics before choosing one.
  - [ ] Define object `get`, `put`, `list`, `delete`, manifest publish, and
    published-manifest read operations, including exact prefix normalization,
    object size limits, content metadata, and conditional-write semantics.
  - [ ] Add parsing, normalization, serialization, duplicate-rejection, and
    illegal-operation tests.

- [ ] **2. Generalize private-volume lifecycle and providers**
  - [ ] Define a provider-neutral private-volume lifecycle for provision,
    inspect, attach/detach, backup, restore, retention, and deletion, including
    capacity limits, lifecycle state, opaque provider handles, and audit
    provenance.
  - [ ] Define a type-specific volume-provider boundary for bounded private
    volumes. Reconcile it with the existing `VolumeStore`,
    `VolumeMetadataRepository`, and `LocalVolumeStore`; do not mistake the
    PostgreSQL metadata adapter for a storage provider.
  - [ ] Replace the implicit zero-or-one per-instance state-volume assumption
    with zero-or-more declared slots and exact revision bindings, including
    optional slots and explicit read-only/read-write attachment modes.
  - [ ] Require an authoritative fenced writer lease for each writable volume
    attachment. Before granting another writer, prove that the old VM/runtime
    detached or was fenced; lease expiry alone is insufficient. If the provider
    cannot prove this, retain recovery state and deny the new attachment. Decide
    read-only concurrency and attachment cleanup semantics before selecting the
    provider API.
  - [ ] Preserve a local raw-file provider as the initial implementation while
    keeping host paths and provider credentials invisible to callers and
    workloads.
  - [ ] Inventory and migrate every legacy reference, including
    `agent_instances.state_volume_id`, `agent_instance_state_volumes`, and
    `agent_instance_volume_leases`; `mailbox_delivery_attempts` volume/lease/
    fencing evidence and its foreign-key, check, and trigger constraints; the
    gateway capability resource-project lookup for `state_volume`;
    `release-domain`'s optional `AgentInstance.state_volume_id`; and related
    operator/admin projections, forced-RLS policies, durable event triggers,
    authorization mappings, and SQL test fixtures. Preserve volume identity,
    data, active leases, ownership, audit/event provenance, and rollback or
    recovery behavior for ambiguous/incomplete rows; stateless instances retain
    zero bindings.
  - [ ] Add real-PostgreSQL and local-provider tests for multi-slot binding,
    exact mount scope, read-only and writable leases, fencing, backup, restore,
    retention, deletion, migration, and failure recovery.

- [ ] **3. Persist and authorize exact resources**
  - [ ] Add authoritative PostgreSQL records for project-owned SQLite databases,
    object buckets, prefixes, manifests, and their lifecycle state, tombstones,
    provider handles, and audit provenance; persist explicit volume slots and
    bindings under the volume lifecycle defined above.
  - [ ] Apply forced RLS and explicit inspect, create, configure, bind, backup,
    restore, publish, and delete permissions.
  - [ ] Enforce project ownership and independent authority to create,
    configure, attach, back up, restore, and delete each private volume, and to
    grant each database or bucket operation to a workload revision. Resolve
    every binding to one exact resource, reject required unbound slots, reject
    undeclared bindings, and deny cross-project bindings unless a specific
    authorized sharing contract exists.
  - [ ] Add OpenFGA/Mélange relations and real-PostgreSQL tests for tenant
    isolation, revocation, lifecycle CAS, concurrent changes, and historical
    provenance.

- [ ] **4. Implement SQLite lifecycle and writer safety**
  - [ ] Define provision, pause, backup, restore, migrate, delete, and
    recovery transitions, including backup consistency and restore provenance.
  - [ ] Use an authoritative fenced writer lease or equivalent serialized lane
    before writable access. Prove prior writer detach/fencing before granting a
    new writer; lease expiry alone is insufficient, and uncertainty must deny
    access while retaining recovery state.
  - [ ] Define WAL mode, checkpoint, fsync, crash, corruption, database-size,
    and migration failure behavior. Never claim multi-writer safety.
  - [ ] Provide an initial SQLite provider that exposes a database only after
    authorization and writer-lane acquisition. It may use the private-volume
    provider internally; keep volume paths invisible to workloads and callers.
  - [ ] Add real-SQLite and real-PostgreSQL failure-injection tests for
    concurrent writers, worker crash, lease expiry, checkpoint failure, backup,
    restore, and stale mount denial.

- [ ] **5. Implement S3-compatible object access**
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

- [ ] **6. Publish immutable content bundles**
  - [ ] Define an immutable manifest containing normalized object keys, content
    hashes, metadata, entry point, and source bucket/prefix provenance.
  - [ ] Require an explicit publish operation to make a manifest eligible for
    static-site or other edge delivery; publishing must not broaden bucket
    access or expose unlisted objects.
  - [ ] Define a future content-edge/CDN provider contract, cache identity, and
    invalidation semantics without implementing CDN configuration in this task.
  - [ ] Add tests proving manifest immutability, hash verification, rollback,
    and no public access before publication.

- [ ] **7. Integrate runtime authority and gateway use**
  - [ ] Include exact SQLite/object bindings, operation ceilings, leases, and
    provider scope in immutable authorization snapshots and runtime sessions;
    include exact private-volume slot bindings and attachment leases.
  - [ ] Give a gateway only the selected database/object operations; deny
    ambient repository, private-volume, mailbox, object-bucket, and Caddy
    authority.
  - [ ] Define how synchronous gateway invocation acquires and releases a
    SQLite writer lane, including timeout and client-cancellation behavior.
  - [ ] Add end-to-end scenarios for a gateway reading/writing authorized
    SQLite state and serving or publishing an authorized immutable object
    manifest.

- [ ] **8. Verify and document**
  - [ ] Document the distinct volume, SQLite, and object resource models,
    provider boundaries, authority and mount limits, writer-lane and fencing
    contracts, migration from implicit state volumes, S3-compatible protocol,
    content-publication semantics, backup/recovery, and deliberate
    durable-object/CDN deferrals.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev check architecture` and resolve new package, SQLx, RLS,
    and dependency-boundary diagnostics without broad exceptions.
  - [ ] Run real-PostgreSQL, real-SQLite, and S3-provider integration and
    failure-injection scenarios.
  - [ ] Run `git diff --check` and `cargo dev quality` for handoff.

## Decisions before implementation

- [ ] Decide whether read-only volume use is a shared mount, snapshot, or brokered
  operation, and define its consistency and revocation behavior. Keep the
  required distinction between declared read-only and read-write authority, but
  do not prescribe a provider API before this decision.
- [ ] Decide whether one resource may satisfy more than one slot in the same
  revision. If allowed, prove that the combined access does not widen the
  authority granted by either slot.
- [ ] Decide whether SQLite access is exposed through a database protocol,
  controlled mount, or another narrow API; specify read-only consistency,
  cancellation, migration ownership, and writer-lane timeout behavior.
- [ ] Select the supported S3 operation subset, consistency/versioning contract,
  multipart rules, credential mechanism, and concrete key/byte/page limits.
- [ ] Agree with the topology task on names, paths, layer/context metadata, and
  which new packages are core contracts/PostgreSQL adapters versus replaceable
  standard providers. Do not change that task's current package-count baseline
  in this task.

## Acceptance criteria

- [ ] Releases can declare zero through multiple named slots. A revision resolves
  each required slot to one authorized same-project resource, permits an
  optional slot to remain unbound, and rejects missing, multiply assigned,
  undeclared, or cross-project bindings unless explicit sharing authority
  applies.
- [ ] Public contracts, release configuration, runtime sessions, and
  authorization snapshots expose no host path, host identity, provider
  credential, or provider-specific configuration; provider adapters can retain
  the local details they require.
- [ ] No second writable volume or SQLite attachment is granted until tests
  prove that the old writer detached or was fenced. Expiry without that proof
  keeps recovery state and denies access.
- [ ] Migration preserves legacy volume bytes, identity, ownership, active lease
  fencing, mailbox execution evidence, authorization lookup behavior, RLS, and
  event provenance; real-PostgreSQL fixtures cover each inventoried reference
  and recovery from ambiguous legacy rows.
- [ ] Cross-project access and each lifecycle/binding permission are checked
  against the exact resource and actor; revocation prevents new operations and
  preserves immutable historical provenance.
- [ ] SQLite failure scenarios prove the selected durability, backup/restore,
  migration, and writer-lane semantics. Object-provider fixtures prove exact
  bucket/prefix confinement, operation limits, credential expiry, immutable
  manifest publication, and no public access before publication.
- [ ] `cargo dev check architecture` and `cargo dev quality` pass with the
  repository's existing strict lint and architecture rules enabled.

## Completion evidence

Record schema and provider versions, legacy-volume migration evidence, exact
slot and mount-scope fixtures, volume lease/fencing/backup/restore/retention/
deletion evidence, capability and RLS fixtures, SQLite writer-lease/crash and
backup/restore evidence, object-scope and credential-expiry evidence,
immutable-manifest publication evidence, end-to-end gateway fixtures, test
counts, and deliberate follow-up tasks for CDN delivery and durable objects.
