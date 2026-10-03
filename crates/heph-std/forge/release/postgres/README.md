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
