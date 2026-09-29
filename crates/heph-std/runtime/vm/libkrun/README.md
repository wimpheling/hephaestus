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

# When

Construct the provider during daemon startup after host configuration and
KVM/cgroup prerequisites have been checked:

```rust
let config = LibkrunConfig::new(runtime_root, image_roots, disk_roots, mount_roots, worker, cgroup_root);
let provider = LibkrunProvider::new(config)?;
```

Pass the provider to `RunOrchestrator`; use `worker_main` only for the dedicated
worker executable, never inside the supervisor process.
