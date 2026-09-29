# Purpose

`heph-secret` prepares the secret view of a run. Just before a guest starts,
it uses the persisted dispatch record to resolve the run's exact leases,
materializes only the approved raw values, and returns a read-only mount that
the VM can attach.

# Responsibilities

Initialization asks the mount provider to validate its dedicated root and
memory-backed policy before any value is materialized. During run preparation,
the manager loads persisted provenance, resolves short-lived leases, passes
approved raw values to the provider, and persists the opaque mount identity
before the VM receives the mount.

Before use, the manager reauthorizes the run's leases. After guest cleanup it
destroys the mount, and after a crash it reconciles orphan directories. These
ordering checks keep a live guest from outliving its secret files, while
lifecycle failures are returned as redacted `RunSecretError` values.

# When

Install `SecretMountManager` as the run orchestrator's `RunSecretManager` with
metadata, dispatch, runtime, and a concrete mount provider. The run
orchestrator then calls `prepare` for each secret-bearing run:

```rust
let manager = SecretMountManager::initialize(
    metadata, dispatch_resolver, runtime_resolver, provider,
    EphemeralSecretConfig { root, require_memory_filesystem: true },
)?;
```
