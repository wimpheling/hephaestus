# Runtime core

Runtime owns the provider-neutral mechanics needed to execute isolated work:
durable runs, VM lifecycle contracts, persistent volumes, exact per-run
workspaces, and mailbox delivery mechanics. The contracts preserve run
provenance and make cleanup explicit after crashes.

## Contracts and decisions

- [`src/`](src/) (`heph-runtime`) re-exports the supported VM, volume, and
  workspace seams without exposing a provider configuration or persistence
  implementation.
- [`vm/trait/`](vm/trait/) defines one-shot VM lifecycle, guest bootstrap,
  bounded events, private service invocation, and `cleanup_orphan` recovery.
  Providers boot the approved `heph-init`; they do not execute an arbitrary
  guest command directly.
- [`volume/trait/`](volume/trait/) defines persistent instance-state volumes,
  exclusive writable leases, fencing generations, host ownership, and
  supervised recovery. Lease expiry is evidence for recovery, not permission
  for immediate reuse.
- [`workspace/domain/`](workspace/domain/) defines exact-commit read-only
  source mounts, separate writable result workspaces, and the host-controlled
  result publication boundary.
- The durable run contracts are in [`../run/`](../run/), and mailbox envelopes,
  state transitions, and dispatch contracts are in [`../mailbox/`](../mailbox/).
  They are part of the runtime mechanics even though those packages retain
  their current physical roots.

An accepted run is tied to its exact release, revision, attachment, repository,
ref, commit, and attempt. The host rechecks live authority before materializing
the runtime and before provisioning the VM. A guest receives an immutable
release tree, an exact source snapshot, and a separate writable result area;
only the trusted importer may publish a result.

## Boundaries and implementations

Core runtime has no libkrun, raw-volume path, local Git process, SQL query, or
NATS topology. Implementations are in [`heph-std/runtime/`](../../heph-std/runtime/),
[`heph-std/run/`](../../heph-std/run/), and the local workspace adapters. The
application composition root selects providers, installs consumers, and owns
startup and shutdown sequencing.

See [`docs/vm-runtime.md`](../../../docs/vm-runtime.md),
[`docs/run-orchestration.md`](../../../docs/run-orchestration.md),
[`docs/releases-and-instances.md`](../../../docs/releases-and-instances.md), and
[`docs/application.md`](../../../docs/application.md).
