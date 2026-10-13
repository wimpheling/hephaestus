# Purpose

`release-postgres` is the PostgreSQL adapter for release publication, reusable
agent instances, capability bindings, repository attachments, update hooks,
and the control-plane UI installation state. It turns provider-neutral release
commands into authorized durable transactions consumed by run and gateway
workflows.

# Responsibilities

The adapter validates and persists release manifests, exact revisions,
immutable artifact references, typed capability ceilings, Git policies,
brokered-secret rules, gateway declarations, and update candidates. Every
command establishes actor context, checks current authorization and optimistic
state, records an event or outbox effect, and commits related rows together.
Update preparation and recovery preserve the old revision until the candidate
hook and its resource cleanup are resolved. UI installation and browser
resource queries apply the same tenant and generation checks, keeping a stale
or unauthorized revision from becoming active.

# When

The explicit Legacy update producer can receive
`with_legacy_vm_placement(scope)`. Fresh hook admission atomically records the
actual configured VM placement and original audited producer actor/request.
It is initially unconsumed; the worker must consume exact birth eligibility
before any resource IO. Replay compares recorded scope and never stamps a plan
onto old queued hooks. This private-schema metadata grants no Source permission,
physical cleanup proof or canonical receipt; default constructor SQL stays
unchanged and application routing remains separate.

Compose `ReleaseService` with the application command and storage adapters when
a release or instance command reaches the control plane:

```rust
let service = ReleaseService::new(pool, authorizer);
```

Call the service inside the command workflow and use its committed result or
recovery decision to drive run, gateway, and event publication.

## Explicit volume compatibility

Authored volume slots are validated, sorted, and frozen into the runtime contract
before its content hash is computed. Empty declarations omit the field and retain
the exact legacy JSON/hash. Legacy state requirements remain separate provenance
and are lifted only in typed effective views; published rows are not rewritten.

Publication accepts these declarations. Legacy import, update candidate creation,
and update hook admission reject nonempty declarations after live project/source
authorization and before replay or metadata effects. Malformed persisted declarations fail closed. Absent
or empty declarations retain the legacy paths.

`import_agent_with_volumes` creates a named instance, its complete immutable
revision, exact declared bindings, and explicit grants in one actor transaction.
It checks live source use, target management, and each volume's attach/grant
permissions before full-input replay. Resolved parameters and sorted selections
are hashed with actor, pins, predicted IDs, policies, and name. Required secret or
generic capability bindings and typed updates remain unsupported. Legacy state
ceilings may select standalone resources as `legacy_declaration` provenance;
no origin pointer or scalar attachment evidence is fabricated.

Named instances keep an independent closed run gate while the runtime profile
is unsupported. Revision runnable state records completeness, and
`instance_volume_status` reports live profile support and removal admission.
Plural runtime materialization, mount safety, and supervised detach/fencing
remain necessary before named instances execute.

Typed volume mount grants are separate from immutable binding evidence and
published generic capability hashes. The actor API audits consumer management
and source grant/attach decisions before creation or replay, commits rejected
authorization audit records separately, and records permanent revocations.
Its transaction helper supports atomic initial import without requiring
an already active or runnable consumer revision. Typed update candidates remain
unsupported. Same-project v1 bindings make source ownership and consumer
management equivalent under the pinned0102 model; no grant is backfilled.

`request_instance_removal` permanently closes future launches under current
consumer management authority, independently of source or original creator
access. It withdraws grants and commits cancellation requests, pauses mailboxes,
and cancels pending delivery/request admission. Existing leased/running attempts,
lease fences, revisions, and external volume references remain retained. Update
recovery cannot reopen the permanent gate. This method records admission only;
trusted VM destruction and complete fenced lease cleanup must be proved before
terminal tombstoning or physical volume deletion is implemented.

Update-hook execution is selected explicitly by `with_execution_profile`.
The ordinary constructor retains public107 SQL regardless of table presence;
`LegacyScalar` writes the fresh legacy profile without an owned plan, while
`OwnedCanonical` commits exact actor/request/command planning admission before
runtime IO. Replay requires live authority and preserves the original profile.

## Configured strict Legacy completion

An explicit `LegacyScalar` service with `with_legacy_vm_placement` validates
worker-only terminal evidence against the actual configured VM owner namespace
and host before update completion/recovery or deferred Normal-drain admission.
The original immutable producer/pins, closed `CleanedUp` Run and global exact
released scalar lease (or qualified genuine no-history boundary) are required.
Released replay only reads history; it never writes the current volume generation
or marks a reused resource Ready. Unknown or foreign history stays held.

This trusts the production Core's qualified scalar terminal transition after
scoped provider cleanup. SQL itself cannot prove physical VM absence, and the
Legacy path creates no canonical cleanup receipt. Canonical receipt checks and
the default public107 constructor remain unchanged. App mixed routing/observer
composition is a later integration task.

### Qualified instance activation

`PostgresInstanceExecutionService` is a separate, explicitly configured actor
adapter. It activates only an exact recipe113 birth whose original protected106
Instance/Create completed successfully. It checks current active caller Manage
and Source Use, the complete immutable107 import, current declared bindings,
grants and Ready metadata, and trusted provider/platform configuration.

The qualified activation adapter requires the private114 qualified activation
schema. Ordinary public107 does not provide these SQL functions, and the
existing default composition is unchanged.

Activation is one-time version1/gatefalse to version2/gatetrue. Exact replay
requires fresh authority and current eligibility; it returns the original
receipt without reopening, updating or importing. Late instance/volume/grant
lock contention is held/transient. Legacy producer configuration and globally
unsupported named Normal/mailbox/update dispatch remain unchanged. Invocation
returns Unsupported without writes in this first adapter group.

Configuration and SQL readiness are not physical provider proofs. Composition
must validate the actual existing VM provider scope and volume root namespace.
Native bound fixtures use real owned registration, Local mkfs/readonly fsck,
first-Ready custody held through original106 completion, and the real107 import;
this proves filesystem prerequisites, not a VM execution or runtime lease.

The shared Core `InstanceExecutionConfiguration` remains reexported from this
adapter. Its checked fields and canonical v1 JSON preserve the existing
configuration stamp; moving the type does not enable any runtime profile.

### Read-only instance safety inspection

`PostgresPreparedInstanceSafetyRepository` lets a freshly authorized current
Remove manager inspect the exact original prepared import, including a committed
import whose original installation ledger is incomplete. The separate worker pool
independently checks immutable creation and permanent removal correlation. Neither
path reconstructs the installer identity, calls import or activation, borrows old
Source access, writes outcome facts, drains a VM or mutates the installation.

Both pools use repeatable read, read-only snapshots. The private118 facades have
fixed search paths and narrow explicit actor/worker execution grants; the worker
receives no broad access to preparation tables. Configuration compares the actual
provider's opaque namespace and host plus the frozen platform input. Scope labels
are backend comparison data and confer no physical ownership.

Inventory includes global Run and lease history and every declared consumer.
Cleanup qualification reuses complete canonical scoped receipt/completion or
qualified Legacy placement and original released tuples. Unknown history is held;
lease expiry, null VM IDs and statuses never establish absence. Other consumers
still require their own removal proof before a volume can be retained.

Authored database cases cover original import commit gaps, source/installer loss,
current Remove reconciliation, permanent closure, substitutions and role denial.
A zero-Run drained case proves only this database observation boundary. Positive
runtime history awaits runtime integration; no fake receipts are seeded. Private
118 adds read-only functions and ACLs only. Default composition, public107, runtime
routing, guarded parent-first removal mutation and an application remover remain
unchanged and outside this adapter slice.

### Qualified prepared-instance closure

A current manager first reads the exact original prepared creation, then admits
permanent closure under a genuine current Remove Drain claim. The adapter uses
the supplied checked intent only after matching the database's immutable project,
intent hash and complete preparation bytes. Historical command provenance is
comparison data; it never becomes an authenticated installer or Source grant.

The owning transaction reserves the full original command and instance set in
sorted order before Project, Deployment and Resource locks. Raw insertions use
try reservations; parent and late instance, Run, volume, lease, grant, mailbox and
event-scope rows use NOWAIT. Any contention rolls back the request, inbox, audits,
events and cancellation writes. The existing generic removal service retains
its authorization and replay behavior; only its identical write tail is shared.

The additive private closure fragment follows the read-only observer fragment
and creates no tables. Default public migrations and startup stay unchanged.
The native target tests real import and ledger correlation, including source loss
and a correlated PostgreSQL advisory-lock wait. A zero-Run closed observation
proves database scheduling/observation, not physical VM cleanup. Fresh manager
takeover while Removing, runtime cancellation execution and the application
removal driver remain separate work.

### Reading a closure after manager takeover

A fresh admitted manager can retrieve the unchanged original permanent instance
request after the old manager or Source is withdrawn. `with_remove_resume` enables
this read-only API only for the observer's matching trusted provider and platform
configuration. The default observer constructor leaves it unsupported.

One repeatable read, read-only actor transaction compares full original preparation,
creation, Drain claim, permanent request and inbox, the current continuation
admission, every current owned cleanup permission and known runtime scope. Unknown
or foreign history remains held. Original actors and request IDs are comparison
data; the current authenticated request stays separate. No extra request, gate
change, receipt, audit, provider operation or physical absence claim is produced.
