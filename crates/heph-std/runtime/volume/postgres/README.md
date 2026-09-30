# Purpose

`volume-postgres` persists the metadata and fencing state required by the local
volume provider. It implements the core metadata repository for volume rows,
exclusive run leases, host ownership, attachment state, heartbeat, and
supervised recovery.

# Responsibilities

The adapter keeps acquisition and state transitions transactional and checks
the current fencing token on attachment, heartbeat, release, and recovery.
Lease expiry is surfaced for a supervisor to fence; it is never treated as
immediate permission for another writer. SQL row conversion returns typed core
values and typed conflicts, stale leases, wrong hosts, or invalid transitions.
The application composition root owns migration timing while this crate keeps
volume SQL inside the declared PostgreSQL adapter.

# When

Construct the repository from the daemon's shared pool and initialize the
schema during startup:

```rust
let metadata = PostgresVolumeMetadataRepository::new(pool);
metadata.initialize().await?;
```

Inject it into `LocalVolumeStore`; the caller receives durable lease decisions
that can be paired with host backing-file operations.
