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

# When

Construct the provider during daemon startup after host configuration and
KVM/cgroup prerequisites have been checked:

```rust
let config = LibkrunConfig::new(runtime_root, image_roots, disk_roots, mount_roots, worker, cgroup_root);
let provider = LibkrunProvider::new(config)?;
```

Pass the provider to `RunOrchestrator`; use `worker_main` only for the dedicated
worker executable, never inside the supervisor process.
