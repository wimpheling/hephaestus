# Purpose

`volume-trait` defines durable persistent-volume and writable-lease contracts
for reusable agent instances. It gives the run workflow a host-neutral way to
resolve instance state, attach it to a VM, renew ownership, and recover after a
crash.

# Responsibilities

`Volume`, `VolumeLease`, and `VolumeAttachment` carry the host owner, backing
identity, filesystem metadata, run holder, expiry, and monotonic fencing token.
`VolumeStore` and `VolumeMetadataRepository` constrain acquisition, attachment,
heartbeat, release, stale-lease discovery, and recovery. Lease expiry is only a
signal for supervised fencing; it is not permission for a second writer to
reuse the disk. Errors distinguish conflicts, stale tokens, wrong hosts, bad
state transitions, and provider failures so recovery can remain deliberate.

# When

Use this crate when composing a persistent state provider or when the run
orchestrator needs an exclusive attachment:

```rust
let attachment = volumes.acquire(volume_id, run_id).await?;
```

Mark the returned lease attached, heartbeat it while the VM owns the disk, and
release it only after provider detachment or an explicitly fenced recovery.
