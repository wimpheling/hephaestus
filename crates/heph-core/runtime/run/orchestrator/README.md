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

Run creation and planned binding are presently separate durable operations. A
crash between them leaves an unresolved run held for reconciliation. Production
composition must close this window with atomic creation/binding or an equally
authoritative persisted admission proof before enabling the path. Historical
rows are never adopted automatically. Application composition, migration
publication, plural dispatch, and native runtime verification remain pending.

Before production enablement, provisioning and cleanup also need a per-VM
physical operation guard and authoritative supervisor quiescence. A durable
pre-provision check alone cannot prevent another recovery process from observing
absence while an asynchronous provision call is still in flight. Cancellation
admission avoids starting that competing cleanup, but cross-process recovery
requires the additional proof before any absence receipt. No application or
migration enablement is authorized by this opt-in checkpoint.
