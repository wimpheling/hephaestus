# Purpose

`heph-run` is the stable runtime facade for starting, cancelling, observing,
and recovering an isolated agent execution. It joins the durable state model in
[`domain/`](domain/) with the lifecycle coordinator in
[`orchestrator/`](orchestrator/) so application composition can depend on one
provider-neutral surface.

# Responsibilities

The facade carries exact instance revision, release, attachment, command, and
run identities into the orchestrator. The underlying workflow acquires state
leases, validates live launch authority, prepares workspaces and runtime
artifacts, provisions a VM, persists bounded events, and cleans every transient
resource. Cancellation and recovery retain the outcome while fencing stale
resources. Persistence, VM, volume, secret, and workspace implementations are
injected through ports so the facade cannot silently bypass their authorization
or cleanup rules.

# When

Use this facade from application or daemon composition when a command has been
accepted and the run needs durable orchestration:

```rust
let run = RunOrchestrator::new(repository, volumes, vm_provider, spec_factory, capacity);
```

Pass `StartRun` and `CancelRun` values through the selected command handler and
retain the resulting `RunState` as the source of truth for lifecycle progress.
