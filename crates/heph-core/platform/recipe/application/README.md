# Recipe deployment application contracts

`DeploymentIntent::new` checks that parsed declaration and resolved graph share
the same recipe identity, version, contract, and declaration hash. It freezes
canonical TOML, resolved JSON, ordinary inputs, exact external identities,
released authority ceilings, dependency order, retention intent, and predicted
resource IDs. UUID v5 resource identities depend only on deployment, resource
name, and purpose. Per-attempt request identity cannot change those IDs.

This crate performs no database, filesystem, VM, or network effects. Planning is
not authorization. A repository implementation must derive release and external
resource views from authoritative adapters, check live project and resource
permissions before replay or effects, audit those decisions, and commit progress
with project events and the existing product outbox. Recipe version one rejects
required secret slots during domain resolution; it stores no credentials or
provider handles.

`DeploymentRepository` admits install/remove, inspects recoverable partial state,
claims resource effects with compare-and-swap versions and fencing generations,
records definite or ambiguous outcomes, reconciles, and finishes commands. Each
claim commits before provider execution. Cancellation, crash, or expired claims
do not prove provider absence and cannot justify another creation or unsafe
storage reuse. Removal drains consumers and proves detach/fencing before deletion.
External volumes can only be verified; retain/delete policy never grants access.
Retained owned resources and removed deployment tombstones stay discoverable.

Validated intent has no deserializer. Database hydration must reparse canonical
declaration source, resolve against authoritative immutable historical release
and external facts, and compare the original fingerprints. Historical inspection
must remain possible after source revocation. Current publication, live resource
facts, and permissions are checked independently before new or resumed effects;
an old receipt cannot authorize them.

Logical command identity is actor/operation/idempotency scoped. Attempt provenance
records a distinct request and attempt ID. Changed input under either a logical
command or project-local deployment key is a conflict. Receipts identify committed
project events; provider errors are represented by closed safe diagnostic codes.


`DeploymentExecutionPreparation` freezes every typed instance import before any
registration, provider or instance effect. It includes the original Install
command, unchanged v1 intent hash, the closed server-selected `RuntimeNamedV1`
profile, and configured platform policy/version even for a zero-instance graph.
Imports retain authoritative source pins, authored runtime-contract hashes,
pinned images, exact released policy, resolved ordinary parameters, full volume
selections, names and stable command/grant IDs. Construction creates no grants
and performs no authorization or IO.

Canonical bytes are bounded to one MiB. Stored input is decoded through private
wire types, reconstructed against the exact historical intent/command, and
compared field-for-field and byte-for-byte, including rederived stable IDs.
`CommandIdentity::from_recorded` reconstructs historical data identity without
synthesizing authentication. An adapter must check real fresh middleware
identity, current target/source/external rights before replay or effects, and
hold execution on any configured policy/version/profile drift. Historical
inspection and removal retain the original bytes without requiring SourceCanUse.
The preparation fingerprint is distinct from the release import inbox hash.

The versioned storage proposal remains private; this core contract supplies no
PostgreSQL adapter, import conversion, engine, dispatch activation or provider
completion proof. Existing per-claim owned-volume preparation remains separate.

The backend `DeploymentExecutionPreparationRepository` separates whole-graph
preparation from admission and provider execution. Its result includes the
complete checked input and a preparation event receipt, rather than an Install
admission or terminal result. A retry must reauthorize the original real actor,
all source pins and selected external bindings before returning historical input.
Manager-only inspection returns data after withdrawal and conveys no authority.

## Retaining a ready owned volume

A current project and volume manager can close attachments, let the known run
owners drain their workloads, verify the original disk without writes, and retain
its data. Removing an installation does not borrow the original installer's
identity or require continued access to its old Source.

The service keeps data held when a consumer, lease, owner, original creation, or
physical disk cannot be checked. Actual journal custody stays owned through the
worker fact and manager's commit, including caller cancellation. Retention leaves
attachments closed; a new consumer needs fresh authorization and an explicit
reopening receipt. The current reopening path requires an originally completed
installation. An unfinished but physically ready birth retains its distinct
positive fact and remains held for reuse.

This opt-in backend path does not initialize roots, format disks, rewrite original
installation receipts, reset lease generations, enable named dispatch, or provide
physical VM shutdown evidence from database status. The checked scope pins come
from the configured provider and remain separate from volume-root ownership.

## Observing an instance before removal

A current manager can inspect an installation's exact original instance import
without replaying the import as its old installer. An independent worker compares
that original input with committed creation, permanent removal, and all work,
Run, lease, grant and consumer history. Source withdrawal does not turn a known
creation into absence or supply the current manager with the installer's rights.

The observer leaves data held when lineage, scope or runtime cleanup cannot be
proved. Its default port is unsupported. A drained observation requires the
owning adapters' exact persisted cleanup evidence; a closed gate, terminal status,
missing VM ID or lease expiry is insufficient. The initial zero-Run database case
makes no physical VM shutdown claim. Actual runtime drain, guarded removal
mutation and application orchestration remain separate boundaries.

### Close a prepared instance before removal

A current deployment manager can load the original instance creation as data,
without replaying import or borrowing the installer's identity or Source rights.
The loader accepts checked immutable deployment intent and compares it with the
stored preparation. Only an exact active original Create or a uniquely proven
completed original Create is eligible; ambiguous history stays held.

The closure port accepts the current admitted Remove Drain claim and permanently
closes future work through a durable removal request and cancellation outbox. It
reserves every original import command and instance before locking parents; late
participants use nonblocking locks. Exact replay preserves the request, closed
gate, versions and events. Closure is scheduling evidence: the separate safety
observer must still qualify actual runtime cleanup before Drain, Detach or Retain.
Adapters that do not implement these ports fail closed. This does not enable an
application removal driver or a new manager's Remove resume while Removing.

### Continue removal under a new manager

A current manager can continue an interrupted removal using a new admitted
command. The original claims and permanent instance closure remain immutable;
the new command carries its own current permissions, version check and audit.
The repository rejects continuation unless its server configuration explicitly
supports the checked preparation and actual provider comparison labels.

The returned old claims and closure are data. They do not authenticate the old
manager, grant provider ownership, prove physical cleanup, or reopen an instance.
This boundary supports original owned Drain, Detach and Retain work. Delete and
terminal Removed inspection remain held in this first adapter slice. A genuine
worker observation is still required before recording a successful outcome.
