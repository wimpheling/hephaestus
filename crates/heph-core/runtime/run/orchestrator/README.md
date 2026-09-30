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
