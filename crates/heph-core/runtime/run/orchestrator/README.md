# Purpose

`run-orchestrator` coordinates one run from durable command acceptance through
guest cleanup. It is the provider-neutral workflow that joins the run state
machine to VM, volume, workspace, runtime-artifact, authority, secret, and
completion ports.

# Responsibilities

Preparation resolves exact runtime provenance, obtains an exclusive state
volume lease when required, prepares source and runtime-Git workspaces, and
checks launch authority immediately before VM provisioning. Provisioning binds
the VM identity, starts the approved guest, and records bounded VM events.
Completion and recovery release mounts, destroy secret material, detach and
fence leases, import only approved results, and preserve redacted failure
diagnostics. The orchestration order makes a stale lease or revoked authority
unable to grant a live guest access to state or secrets.

# When

Use this crate from the application composition root after the required ports
are available:

```rust
let orchestrator = RunOrchestrator::new(
    repository,
    volumes,
    provider,
    spec_factory,
    instance_state_capacity_bytes,
);
```

Install the workspace, runtime, authority, secret, and completion managers on
the builder before dispatching `StartRun`.

## Durable cleanup (opt-in)

`with_cleanup_repository(cleanup_repository, run_volume_store)` enables the
complete-set cleanup protocol. A new run binds the actual provider's persistent
owner namespace, host, and planned VM ID before preparation or volume IO. An
unsupported owner fails before acquisition. This opt-in does not enable named
dispatch or replace the current scalar preparation and heartbeat paths.

Cleanup closes acquisition and snapshots every durable lease and fence,
including an explicit empty set. It reloads a recorded receipt before repeating
physical destruction. An active handle must match the exact VM; scoped provider
confirmation is required after destruction and for recovery without a handle.
Transient workspace, authority, secret, and runtime cleanup follows the recorded
observation. Only atomic repository completion releases the full fence set and
permits `CleanedUp`, followed by completion callbacks. Physical or transient
failure holds all fences. Recovery groups stale leases by run and never releases
them through the scalar recovery methods. Its count is completed runs.
Physical destruction and scoped confirmation share a 30-second deadline;
`with_cleanup_timeout` can shorten it. Timeout records no receipt and retains
the active handle for reconciliation. A missing handle requires authoritative
scoped absence; successful exact-handle destruction records destruction.

Historical missing VM or owner scope stays unresolved; the current host or run
UUID never supplies missing evidence. Already-persisted historical `CleanedUp`
runs without receipts retain idempotent callbacks only after a global query
confirms no held lease. Completed receipt replay performs no destruction and
cannot release a newer consumer's fence. Cancellation admission requests stop;
the active start worker owns physical cleanup, preventing a competing absence
observation while provisioning is in flight.

The opt-in path requires `create_run_with_vm_plan`: creation commits the exact
provider ownership and VM ID with the command before preparation. Adapters
without this operation fail closed. Precreated update hooks require immutable
fresh admission that prohibits IO until exact worker consumption; historical
queued rows receive no admission by inference. Application composition,
migration publication, plural dispatch, and native runtime verification remain
pending.

The opt-in path now holds a per-run operation guard from the final durable open
check through asynchronous provisioning and active-handle registration. Launch
repeats the authoritative open check under the same guard before transitioning
to `Starting`, and keeps it until the Start RPC returns. Cleanup closes
acquisition first, then waits for that guard before receipt lookup, destruction,
or authoritative absence. Waiting and physical confirmation share the cleanup
deadline. Already-in-flight provisioning/Start may finish after closure; cleanup
cannot certify absence before those calls quiesce. No DB transaction spans IO.

A separate live-start claim lasts through each start future. Duplicate starts
are rejected before preparation and do not clean another worker's resources.
Weak registry entries are reclaimed only after holders and queued waiters drop
their exact guard/claim. Different runs use different operation mutexes.

Composition must provide one shared orchestrator/registry for each exclusively
supervised provider owner. Libkrun's version 2 lifetime supervisor lock excludes
another process/provider for that owner; providers without equivalent ownership
remain unsupported. The operation guard is released before reentrant failure
cleanup and never spans workload execution. Application enablement and plural
runtime integration remain separate checkpoints.

## Complete-set preparation and live execution (opt-in)

`with_volume_preparation()` additionally enables preparation from the exact
persisted revision's zero-to-32 selection set. It requires canonical cleanup.
There is no `StartRun.requires_state` acquisition branch in this profile. Every
attachment must match the selected run, release, revision, slot, resource,
project, host and capacity; refreshes must preserve every acquired lease ID and
fence. Empty sets remain explicit. Only an exact singleton `LegacyOrigin`
projects the historical scalar tuple. Both proven `LegacyOrigin` and
`LegacyDeclaration` produce protocol 11 typed `BuiltinStateSQLite` metadata
with the frozen legacy requirement marker; canonical specs use no scalar state
labels. Named slots never supply scalar pointers. Ordinary explicit SQLite slots
use `None`, even if their name/path resembles legacy state, and the application
owns their database. Default scalar preparation retains its compatibility labels.
App factories must recheck the frozen release requirement before enabling this
profile; guest validation and initialization are separate from mount authority.

Factories must implement `build_with_volumes`; caller/source adapters must
implement `authorize_with_volumes`. Both defaults reject this profile, even for
zero volumes. The authorizer must prove live execute/update authority and use
of the exact released source. Mailbox adapters must resolve their authoritative
accepted/delegated producer evidence; no user is fabricated from an attachment
or event. Mount preflight independently checks each live typed issuer/grant.

Caller/source, secret, runtime capability and whole-set mount checks share one
two-second deadline before acquisition and immediately before Start under the
operation guard. An independent one-second monitor races provisioning, Start,
guest acknowledgement, event persistence and workload completion. Its complete
lease refresh also runs for zero volumes. Stalled event writes or missing guest
acknowledgement cannot starve live checks. A successful check may take up to two seconds; an earlier successful snapshot,
the next one-second poll and a new two-second check fit the five-second detection
bound. A positive scoped physical cleanup
observation ends monitoring; an absent in-memory handle cannot end it.

After successful guest completion, the private owned drain phase skips mount
renewal/preflight because its own acquisition closure is intentional and every
fence remains held. Independent caller/source/runtime/secret checks continue
within the same two-second deadline until scoped physical confirmation. External
closure before this phase remains terminal; VM exit alone does not end checking.
Cleanup snapshot setup is bounded to two seconds; scoped physical confirmation
remains bounded by the cleanup timeout. Post-confirmation bookkeeping has a
separate four-second bound.

On live-check failure, the execution future is dropped before bounded physical
revocation under the operation guard. Exact active-handle destruction and scoped
confirmation use the captured, durably checked VM ID and actual owner scope.
This physical step precedes database cleanup bookkeeping and creates no receipt
or lease release. Physical uncertainty retains the handle and every fence.
Database failure after physical confirmation still retains all fences until the
normal durable target/receipt/atomic-finish protocol succeeds. Bookkeeping is
bounded separately and exposes incomplete cleanup for recovery.

The scalar profile remains the default. App composition and named database
admission are not enabled here. Before enablement, the app authority adapter
must support exact live `Starting`/`Running` pins and permanent closure, mailbox
producers need positive fresh VM-plan admission, and caller/source adapters must
implement the stronger plural authorization port. Guest protocol 11 and native
full-runtime verification remain separate work. Core fake-port tests establish
ordering and failure behavior, not physical VM or PostgreSQL enforcement.
