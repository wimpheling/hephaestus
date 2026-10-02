# Resource recipes for private volumes, SQLite, PostgreSQL, and S3

Owner: resource-recipes implementation

## Current implementation status

Implementation is in progress on [PR #60](https://github.com/wimpheling/hephaestus/pull/60).
The first delivery is local application/SQLite install, inspect, and remove.
PostgreSQL and hosted S3 recipes remain dependent on private networking.

Verified checkpoints:

- `recipe-domain` validates the bounded version-1 static TOML graph, release
  pins, inputs, exact volume slots, external references, and removal policy.
  `volume-domain` defines validated paths, capacities, and RO/RW declarations.
- `recipe-application` defines immutable deployment intent, stable predicted
  resource IDs, lifecycle states, and repository/effect/reconciliation ports.
  These ports alone do not execute deployments.
- Release configuration and PostgreSQL catalogs persist authored named-volume
  slots. Empty declarations preserve captured historical configuration and
  runtime-contract hashes. Legacy import and update paths reject explicit slots
  until typed runtime materialization is available.
- Focused strict Rust checks, architecture checks, and real PostgreSQL
  publication/import/update regressions passed for these checkpoints.
- Additive migrations 0101/0102 generalize the existing volume table to stable
  project ownership and add exact immutable revision-volume bindings. Real
  actor/RLS checks and a populated schema-0100 upgrade rehearsal preserve
  historical checksums, volume metadata, active leases, and mailbox fencing
  evidence. This rehearsal verifies database records and backing references,
  not filesystem bytes. Historical authorization generation is frozen;
  current authorization diagnostics pass with zero warnings.
- Guest named-volume contracts (`0fee5224`) passed 107 focused tests. Guest
  protocol version 10 rejects stale initializers before execution. A real KVM
  guest check (`399979fe`) passed with privileged RO/RW controls, persisted RW
  data, immutable RO backing bytes after confirmed VM destruction, and refusal
  of dirty read-only filesystems before workload execution. This checks the
  guest/provider boundary, not installed recipe execution.
- Local provider provisioning (`64e96dcb`) passed real ext4 creation and
  recovery checks, subprocess lock lifetime checks, actor/RLS tests, an upgrade
  of a populated database from schema 0102 to 0103, and legacy lease/recovery
  regressions. Existing backing bytes and immutable UUID/capacity reservations
  remain protected.
- Deployment persistence (`9c5dc4b6`, `3087ddbe`) passed 10 real PostgreSQL
  admission, inspection, and removal admission cases plus focused strict checks.
  This checkpoint records authorized intent and progress.
- Exact consumer volume grants (`71ad46b1`, migration 0105) passed real
  PostgreSQL authorization and populated upgrade checks. Grants pin the
  revision, export, slot, volume, mode and contract hash; mount checks retain
  live issuer authority and permanent revocation. Runtime enforcement and
  physical revocation require the remaining orchestrator integration.
- The execution ledger (`5ef6901f`, migration 0106) passed 15 real PostgreSQL
  execution cases and 10 admission regressions, including a populated upgrade.
  Durable claims, CAS generations, immutable results and worker-only outcome
  receipts support cleanup by a current manager after original actor/source
  access loss. Fixture receipts prove ledger semantics; real provider adapters
  still need to prove creation ownership, readiness, destruction and detach.
- The [local SQLite example](../../../../examples/local-sqlite/README.md)
  (`401c64fb`) includes a pinned-builder application, bounded recipe inputs and
  a distinct external-volume reuse recipe. Fourteen actual Python tests and one
  Rust parser/resolver integration test passed, with focused strict checks.
  Parser release fixtures are not publication or deployed VM evidence.
- Typed instance lifecycle (`55063387`, migration 0107) passed actual
  PostgreSQL import/removal, actor/RLS, concurrent grant/closure, and populated
  schema-0106 upgrade checks, preserving legacy hashes, leases and mailbox
  fences. Initial import atomically records exact bindings and grants;
  permanent removal closes admission and revokes grants. Named dispatch stays
  unsupported, and removal admission does not prove physical cleanup.
- Canonical run cleanup contracts (`eff9758b`) passed 19 domain tests
  (16 new cleanup cases), 13 existing orchestrator integration tests, focused
  strict Clippy/docs, formatting and architecture checks. Targets bind the
  exact provider/host/VM and complete lease/fence set, including an explicit
  empty set. These are additive core contracts; worker persistence, verified
  destruction and atomic whole-set lease release remain to be integrated.
- Authoritative planning (`bc542fe5`) reloads published releases and pinned
  images, checks live permissions and exact bindings, and derives stable
  project-scoped deployment IDs. Twelve real PostgreSQL planning cases and ten
  admission regressions passed. Planning itself performs no provider effects.
- Opt-in canonical run cleanup (`61490201`) closes acquisition, confirms the
  exact scoped VM, records a complete-set receipt, and releases every matching
  lease atomically before completion callbacks. Twenty-nine orchestrator tests
  and twenty domain tests passed with strict checks. Its PostgreSQL adapter is
  tested on an unpublished migration chain; application composition, atomic
  run ownership admission and plural execution remain pending.
- Owned backing provisioning (`a85f3da5`) records the exact root namespace,
  inode birth and formatting intent, pins filesystem descriptors, and rechecks
  the original actor's authority before formatting. Fifty-four filesystem
  tests passed. Three combined real PostgreSQL/local-provider tests also passed
  with strict Clippy, including readonly recovery after a missing final record
  and actor permission loss, and refusal of contradictory or foreign backing.
  Migrations 0108–0111 remain unpublished. These checks establish the provider
  boundary; they do not run an installed SQLite application.
- Provisioning and cleanup coordination (`d6ad7fe6`) serializes each run's VM
  operations and excludes a second supervisor for an owned provider root.
  Cleanup waits for in-flight provisioning or Start before confirming absence.
  Thirty-six orchestrator integration tests, two guard tests, seven VM-trait
  tests and 97 libkrun library tests passed with strict checks. Real owned-VM
  validation and atomic run creation remain separate work.

Provider orchestration and ownership seals, plural runtime attachment and cleanup,
physical revocation, authenticated project web UI commands, real SQLite
publication/install/inspect/remove and retained-data recovery, and the full
repository quality gate remain unfinished. The existing authenticated web UI is
the first entry point; there is no normal authenticated Heph CLI yet. Backup
metadata fields do not implement backup: volume backup and restore remain
unimplemented and required by the broader checklist below.

Project membership and deployment history grant no resource authority. The
initial volume profile rejects sharing, including read-only sharing, until its
consistency contract is defined. This status covers steps toward the first
usable local delivery; the broader service and networking checklist remains open.

## Outcome

Let users install, inspect, and remove versioned resource recipes that compose
Heph infrastructure into useful application services. Bundle SQLite,
PostgreSQL, and S3 conveniences in the distribution, with UI and defaults,
using reusable infrastructure contracts and replaceable standard providers.
This TODO specifies intended work; it does not claim these recipes exist.

Core contracts cover instances, bounded private volumes, service endpoints,
secrets, resource requirements, authorization, and exact bindings. Standard
providers implement those contracts. The distribution supplies convenience
recipes. Adding a recipe does not automatically introduce a core resource kind
or require a universal storage trait. Decide parser and execution placement
across core, standard providers, and applications before implementation.

## Recipe contract

A recipe is a bounded declaration of a static resource graph. Its minimum
contract includes:

- Immutable recipe identity and version, plus explicit compatibility with a
  versioned recipe contract and infrastructure APIs.
- Typed, validated inputs with bounded defaults, and uniquely named resources
  using known resource kinds and legal operation combinations.
- Explicit dependencies, resource bindings, and bindings to declared release
  slots. Required slots resolve exactly once; optional slots may be unbound.
- Declared outputs conveying exact resource references or controlled resolved
  configuration. Outputs grant no ambient authority.
- Per-resource removal policies distinguishing retain from delete. Retaining
  durable data is the default; deletion requires explicit policy and authority.

References are bounded literals, typed inputs, and named resource references.
Reject loops, recursion, general expressions, and conditional workflows. Pin
software releases immutably. Initialization and application migrations execute
inside packaged workloads with authority over their own data. A recipe cannot
execute host hooks or migrate Heph's internal database.

Install, inspect, and remove are the initial lifecycle. Upgrade compatibility,
data migration, and rollback need a separate deferred contract. Reinstall or
upgrade must never silently replace an existing deployment or migrate its data.

## Conceptual compositions

These graphs illustrate bindings, not a proposed serialization schema.

| Convenience | Resource graph and consumer authority |
| --- | --- |
| SQLite | Application release → named private-volume slot → bounded volume mounted at a controlled guest path. SQLite is embedded application software. Read-only/read-write attachment authorizes file bytes; it does not enforce per-query or per-table policy. |
| PostgreSQL | Pinned server release → instance + volume + secret credentials + private TCP endpoint. The consumer independently needs an authorized network connection and database credentials. Database permissions are enforced by the server. |
| S3 | Authorized service endpoint + bucket/prefix scope + explicit operations + scoped credentials or broker. The distribution may provision a released server or bind a configured external provider. |

SQLite needs no new SQL provider or database resource kind by default. Exclusive
writable volume attachment protects an attachment, not a blanket SQLite
single-connection rule; application software owns SQLite concurrency and
durability. Backups require an application-consistent snapshot or application
quiescence/SQLite-aware preparation where needed, without adding SQL operations
to core's volume API.

S3 provisioning and security may require an explicit typed provider/resource
contract. Record that decision from actual requirements before implementing
it; recipes cannot invent dynamic plugin resource types. Specify the supported
S3 operations, bucket/prefix normalization, consistency/versioning, multipart
behavior, byte/key/page limits, and credential mechanism at that boundary.

## Locked decisions

| Area | Decision |
| --- | --- |
| Ownership | Each resource has one stable owning project. Legacy state volumes inherit their instance's project. Project membership alone grants no use or lifecycle authority. |
| Authorization | Authorize the caller for every lifecycle action and each exact resource/operation. Grant consumers only their exact bindings. Same-project binding is required; sharing needs a separate explicit contract. |
| Release slots | Zero through multiple uniquely named slots declare bounded requirements and read-only/read-write modes. Immutable revisions bind required slots exactly once; reject undeclared and duplicate assignments. No implicit instance-wide volume fallback. |
| Existing volume contracts | Extend `VolumeStore`, `VolumeMetadataRepository`, and `LocalVolumeStore`. The PostgreSQL metadata adapter is not a storage provider. Preserve the local provider initially. |
| Attachment safety | At most one writable attachment per volume. Prove the old writer detached or was fenced before granting another; lease expiry alone is insufficient. Uncertainty retains lease/recovery state and fails closed. |
| Configuration boundary | Stable typed resource IDs, bounded claims, controlled guest mount paths, and service endpoint URLs may appear in resolved workload configuration. Host IDs/paths, raw provider admin credentials, and general host filesystem authority remain internal. |
| Authority ceilings | Recipe declarations and deployment snapshots bound authority but do not confer it. Recheck live authorization on resume and before new effects; historical records never grant indefinite rights. |
| External resources | Record resources created and owned by the deployment separately from explicitly external/adopted resources. Recipe removal never deletes external/adopted resources. |
| Extensions | Third-party recipes use the same validator and permissions. Extensions/apps own their data migrations and consume versioned APIs; they cannot define arbitrary resource kinds or bypass provider boundaries. |
| Runtime revocation | Specify bounded revocation for mounts, established network flows, and credential expiry at each provider. Do not assume per-operation authorization checks on already mounted files. |

## Execution and durable recovery

Validate the dependency graph separately from authorization: reject cycles,
duplicate names, missing/unknown references, incompatible bindings, and illegal
operation combinations before effects. Never infer access from graph reachability.

Persist a durable deployment record with recipe identity/version, validated
configuration, secret references without plaintext secrets, immutable resolved
graph, resource IDs, release-slot bindings, authority ceilings, progress, and
ownership/removal provenance. Provider handles remain internal. Historical
records support audit and recovery without becoming reusable grants.

An explicit deployment/request identity makes install retries idempotent:
resume the same deployment without duplicate resources or consumer grants.
Multiple providers cannot promise an atomic install. Persist partial progress,
reconcile ambiguous provider outcomes, and expose dependency status and failures
through inspect. When the provider cannot prove whether creation succeeded,
retain recovery state and require reconciliation before retrying that creation.
Cancellation and process crashes leave recoverable records.

Removal drains consumers, detaches mounts, and proves fencing where needed
before deletion. It cleans up deployment credentials and connections, respects
retain/delete policy, and resumes safely after failure. Retained resources stay
discoverable with stable ownership and provenance for authorized recovery or
reuse; they must not become orphaned records or hidden data.

## Dependencies and sequencing

- [Agent principals and runtime authority](../../../done/mvp-01-agent-principals-capabilities-and-runtime-authority.md).
- [Durable mailboxes and stateful dispatch](../../../done/mvp-02-durable-agent-mailboxes-and-stateful-dispatch.md).
- [Event ingress and routing](../../../done/mvp-03-event-ingress-and-caddy-routing.md).
- [Distribution platform](../../distribution/define-own-the-loop-agent-platform.md).
- [Grant-controlled private VM networking](../grant-controlled-private-vm-networking.md)
  is required for PostgreSQL and self-hosted S3 service recipes. The local
  volume/SQLite phase can ship first with its own acceptance evidence.
- Coordinate package placement and architecture metadata with the completed
  [workspace topology migration](https://github.com/wimpheling/hephaestus/pull/57).

## Scope and follow-ups

This task delivers reusable volume infrastructure and bounded recipe lifecycle,
then service compositions as their dependencies become available. General host
filesystem access, network filesystem shares, distributed database replication,
and unrestricted cloud credentials are outside its contract.

Immutable content manifests, public publication and CDN delivery, durable
actors, and a managed SQL query API with database/table permissions remain
explicit follow-ups. They are not implemented by mounting SQLite files or
installing a service recipe. Gateway consumers use declared mounts/endpoints
and exact grants under the same infrastructure contracts.

## Implementation checklist

- [ ] **1. Decide boundaries and define the recipe contract**
  - [ ] Record engine/parser placement, API/contract versions, supported kinds,
    immutable recipe/release identity, input validation, and output semantics.
  - [ ] Define bounded references, graph validation, resource requirements,
    release-slot bindings, and retain/delete policy; reject unsupported syntax.
  - [ ] Validate guest mount paths and endpoint URLs as controlled configuration;
    reject host paths and provider admin credentials in public contracts.
  - [ ] Specify create/bind/use/inspect/remove authorization independently,
    including exact consumer grants and explicitly external resource bindings.

- [ ] **2. Generalize private volumes without losing existing state**
  - [ ] Extend existing volume contracts for bounded provision, inspect,
    attach/detach, backup, restore, retention, and delete lifecycle records,
    including capacity, stable IDs, internal handles, and audit provenance.
  - [ ] Replace implicit zero-or-one state volume with zero-or-more named
    release slots, exact immutable revision bindings, and explicit RO/RW modes.
  - [ ] Specify read-only sharing/snapshot consistency and cleanup. Enforce
    exclusive writable attachments with proven prior detach/fencing; preserve
    recovery state when the provider cannot prove safety.
  - [ ] Inventory and migrate `agent_instances.state_volume_id`,
    `agent_instance_state_volumes`, and `agent_instance_volume_leases`;
    `mailbox_delivery_attempts` volume/lease/fencing evidence and its foreign-key,
    check, and trigger constraints; gateway resource-project lookup for
    `state_volume`; and `release-domain`'s optional `AgentInstance.state_volume_id`.
  - [ ] Migrate operator/admin projections, forced RLS, durable event triggers,
    authorization mappings, and SQL fixtures. Preserve bytes, identity, owner,
    active leases/fencing, event provenance, and recovery for ambiguous rows;
    stateless instances retain zero bindings.
  - [ ] Verify local-provider behavior and real-PostgreSQL metadata migration,
    including multi-slot scope, backup/restore, fencing, and retained data.

- [ ] **3. Persist and reconcile deployments**
  - [ ] Add authoritative deployment/progress records and exact resource/slot
    bindings, secret references, ownership provenance, policy, and tombstones.
    Keep provider handles internal and plaintext secrets out of deployment records.
  - [ ] Enforce forced RLS and exact authorization, retaining historical
    evidence while rechecking live authority on resumed/new effects.
  - [ ] Implement idempotent install with explicit identity, durable progress,
    ambiguous-result reconciliation, and inspectable dependencies/failures.
  - [ ] Recover cancellation/crashes and partial multi-provider failures without
    duplicate resources, widened grants, or unsupported atomicity claims.
  - [ ] Implement resumable remove with drain/detach/fencing, credential and
    connection cleanup, safe retain/delete, and external resource protection.

- [ ] **4. Bundle and exercise the local SQLite composition**
  - [ ] Package a pinned application release using a declared volume slot;
    expose controlled guest configuration and exact attachment authority.
  - [ ] Run initialization/migrations in the workload over its own data. Specify
    SQLite/WAL/crash behavior and consistent backup preparation in the package.
  - [ ] Exercise install/inspect/remove, RO/RW mounts, multiple application
    connections where supported, backup/restore, and retained-data recovery.

- [ ] **5. Add service recipes after networking is available**
  - [ ] Compose PostgreSQL from a pinned server instance, volume, secrets, and
    private TCP endpoint; require independent consumer connection and database
    credentials. Keep server data migrations inside its packaged workload.
  - [ ] Record whether S3 needs a typed provisioning/security contract, then
    support a hosted released server and/or a configured external provider.
  - [ ] Scope S3 to endpoint, bucket/prefix, and supported operations using
    short-lived credentials or a broker; implement documented limits and errors.
  - [ ] Verify credential delivery/expiry, prefix confinement, connection
    revocation, partial provisioning recovery, and external-provider removal.

- [ ] **6. Integrate distribution UI and complete verification**
  - [ ] Bundle versioned recipes/defaults and expose authorized install,
    inspect/status, retained resources, failures, and removal policy in the UI.
  - [ ] Document provider boundaries, application migration ownership, bounded
    revocation, legacy migration, recovery, and deferred upgrade/SQL/CDN/actor work.
  - [ ] Run `cargo fmt --all -- --check`.
  - [ ] Run `cargo clippy --workspace --all-targets --all-features`.
  - [ ] Run `cargo test --workspace --all-features`.
  - [ ] Run `cargo doc --workspace --all-features --no-deps`.
  - [ ] Run `cargo dev check architecture` and resolve diagnostics under existing
    SQLx, RLS, transport, committed-outbox, and sensitive-field boundaries.
  - [ ] Run focused real-provider integration/failure checks, `git diff --check`,
    and repository handoff gate `cargo dev quality` with strict rules enabled.

## Open implementation decisions

- [ ] Place recipe parsing, orchestration, persistence, and distribution bundles
  within the architecture; choose packages and metadata before implementation.
- [ ] Decide whether one resource may satisfy multiple slots; if allowed, prove
  combined use stays within each grant and attachment constraints.
- [ ] Select read-only volume consistency, backup preparation, and provider
  mechanisms that bound mount revocation and prove writer detach/fencing.
- [ ] Specify endpoint/established-flow revocation and workload secret delivery,
  rotation, expiry, and cleanup without exposing provider admin credentials.
- [ ] Select S3 contract necessity, supported operations and numeric limits,
  external-provider binding, scoped credential/broker mechanism, and recovery.
- [ ] Define idempotency identity lifetime and ambiguous provider reconciliation,
  including retry after revocation and authorized reuse of retained resources.

## Acceptance criteria

- [ ] Recipe validation rejects cycles, duplicate/missing/unknown references,
  unsupported syntax/kinds, incompatible operations, and invalid slot bindings
  before effects. Immutable recipe/release pins and API compatibility are checked.
- [ ] Zero-to-multiple slots bind exact same-project resources with explicit
  authority. Required/optional slots, caller lifecycle rights, and consumer
  mount/network/credential rights are independently tested; membership is insufficient.
- [ ] Public configuration allows controlled guest paths/endpoints and contains
  no host paths, host identity, raw provider admin credentials, or plaintext
  durable secrets. Third-party recipes cannot exceed the same validator/grants.
- [ ] A second writable attachment requires proven old-writer detach/fencing.
  Expiry alone denies access and preserves recovery evidence; SQLite tests do
  not impose a blanket single-connection restriction or claim SQL-level policy.
- [ ] Legacy migration fixtures cover every inventoried reference and preserve
  bytes, identity, ownership, active fencing, mailbox evidence, RLS, authorization
  lookup, and event provenance, including recovery from ambiguous rows.
- [ ] Retrying install after failure/cancellation/crash resumes one deployment
  without duplicates. Partial provider outcomes are visible and reconcilable;
  revoked callers cannot use historical snapshots to continue new effects.
- [ ] Removal safely drains/detaches/fences, cleans credentials/connections,
  retains discoverable data by default, and never deletes external/adopted
  resources. Failed removal resumes safely under current authorization.
- [ ] Provider evidence establishes consistent backup/restore, bounded mount and
  network revocation, S3 bucket/prefix/operation scope, and credential expiry.
  Consumers cannot obtain database access from network authority alone.
- [ ] Install never blindly upgrades/reinstalls an existing deployment. Extension
  migrations affect only owned application data; follow-ups remain explicit.
- [ ] Architecture checks and `cargo dev quality` pass for implemented phases
  with strict Rust/Clippy/rustdoc rules enabled; phase status records dependencies.

## Completion evidence

Record implemented phases, contract/schema/provider and pinned recipe/release
versions, boundary decisions, legacy migration fixtures, exact slot/authority
tests, fencing and bounded revocation evidence, backup/restore and retained-data
recovery, idempotent retry/crash/cancellation and partial-failure scenarios,
external-resource protection, scoped S3 credentials, UI lifecycle evidence, and
quality-gate results. Link deferred upgrade/migration, managed SQL permissions,
content publication/CDN, and durable-actor tasks without claiming completion.
