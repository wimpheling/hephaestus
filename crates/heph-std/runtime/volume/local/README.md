# Purpose

`volume-local` manages the host-side raw file for one persistent agent-state
volume. It combines the core volume contract with injected metadata so the
single-host filesystem and the durable lease repository remain separate.

# Responsibilities

`LocalVolumeStore` validates absolute non-overlapping roots, host identity,
lease duration, minimum capacity, and direct `.raw` backing paths. It creates or
resizes the file, formats it with the recorded filesystem UUID using
`mkfs.ext4`, and marks metadata ready before returning an attachment. Acquire,
heartbeat, release, stale-lease discovery, and recovery delegate ownership and
fencing to the metadata port. Root checks prevent a configured path from
overlapping transient VM state or escaping the persistent volume directory.

# When

Initialize the store during host composition, then use it as the orchestrator's
`VolumeStore`:

```rust
let store = LocalVolumeStore::new(metadata, config)?;
store.initialize().await?;
```

The caller receives a formatted volume or an explicit backing or lease error;
database migrations and durable fencing remain the metadata adapter's job.
