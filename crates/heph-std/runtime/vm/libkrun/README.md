# Purpose

`vm-libkrun` runs Hephaestus guests as isolated Linux microVMs on a prepared
Fedora host. It adapts the core VM specification to libkrun/libkrunfw, KVM,
passt networking, virtio-fs mounts, runtime bridges, and the approved guest
initializer.

# Responsibilities

`LibkrunProvider` validates host roots, executable and device paths, service
identity, cgroup-v2 limits, and worker timeouts before provisioning. Each VMM
lives in a dedicated worker process; the parent communicates over framed IPC,
while the worker owns libkrun handles, guest sockets, and cleanup. The provider
restricts root filesystems, disks, and mounts to canonical configured roots,
applies CPU, memory, PID, I/O, disk, and wall-clock limits, and keeps secret
and runtime-Git bridges on their dedicated channels. Readiness, bounded events,
exit status, and orphan cleanup return through the core VM contract.

`LibkrunProvider::new_owned(config, host_id)` explicitly enables scoped cleanup.
It exclusively writes and fsyncs a private random owner GUID bound to the
runtime directory and delegated cgroup directory device/inode/UID plus the
configured host. A managed restart retains that owner. Unclassified VM resources,
copied markers, symlink roots or metadata, changed hosts and replacement roots
fail closed; automatic historical adoption is unavailable. Roots must belong to
the configured service UID and deny group/world writes.

Validation and physical operations hold shared owner guards; initialization alone
holds an exclusive guard. Independent VM operations can proceed together.
Provisioning revalidates ownership before returning a VM and destroys its exact
worker and pinned allocation roots if ownership changed during allocation.
Failed destruction retains the exact worker/resource handle and refuses VM ID
reuse. Scoped recovery rechecks ownership and confirms termination/reap before
releasing that retained handle; uncertain cleanup never reports success.
Scoped cleanup pins both directory handles through IO, rejects registered live
instances, and revalidates ownership before confirming destruction or absence.
An unchanged pathname or missing VM directory never establishes another root's
ownership. A changed cgroup root also requires explicit future reconciliation;
there is no implicit ownership rollover across host reboot. The ordinary `new`
constructor preserves existing provisioning and reports ownership unsupported.
Owned providers also apply their ownership checks to unscoped orphan cleanup.

Named ext4 attachments preserve the disk's read-only backend flag and validate
its filesystem UUID on the exact selected raw file before launch. The guest
rejects duplicate UUID devices, checks the kernel device mode, and mounts
read-only data with `MS_RDONLY` after rejecting known dirty journal state.
It does not initialize SQLite or change ownership on read-only data. Mount
directories are opened component by component without following symlinks;
ancestors must be protected from replacement by the guest user. Cleanup
rechecks filesystem and kernel mount identity, attempts reverse unmounts, and
reports unresolved failures for host destruction before attachment release.
Empty named lists preserve the legacy wire shape and scalar state-volume path.
Protocol version 10 rejects stale initializers that could ignore named fields;
rebuild the initializer and cached guest roots together. The 107 focused tests
verify contracts and bootstrap helpers; they provide no native KVM evidence.
This slice does not implement run authorization or durable plural attachments.

## Native named-volume proof

Run `scripts/run-libkrun-named-volumes.sh` on the prepared KVM host. It builds
fresh protocol 10 binaries and exports a disposable root from Ubuntu pinned at
`sha256:52df9b1ee71626e0088f7d400d5c6b5f7bb916f8f0c82b474289a4ece6cf3faf`.
The test-only root probe uses the trusted OCI builder path to exercise device
ioctls, remounts, and raw writes; ordinary workloads receive no additional
authority. The fixture leaves historical image caches and volumes untouched.

The native test verifies named paths and UUIDs, kernel read-only flags, denied
file/raw writes and writable remounts, unchanged read-only backing bytes after
VM destruction, persisted writable data, and dirty-filesystem rejection before
workload execution. Equivalent privileged writable controls must succeed.
On libkrun 1.19 the read-only device accepts `BLKROSET(0)` but its effective
read-only flag remains set and the altered raw write is denied. The test records
this distinction rather than assuming the ioctl itself fails. Its output records
the exact initializer, probe, and worker hashes and confirmed destruction.

# When

Construct the provider during daemon startup after host configuration and
KVM/cgroup prerequisites have been checked:

```rust
let config = LibkrunConfig::new(runtime_root, image_roots, disk_roots, mount_roots, worker, cgroup_root);
let provider = LibkrunProvider::new(config)?;
```

Pass the provider to `RunOrchestrator`; use `worker_main` only for the dedicated
worker executable, never inside the supervisor process.

The owned constructor is not yet wired into application startup or run cleanup.
No existing runtime path layout changes in this slice. Unit tests cover ownership
and emulated cleanup; native scoped VM destruction remains separate evidence.
