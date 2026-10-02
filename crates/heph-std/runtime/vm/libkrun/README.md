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
constructor preserves provisioning on unclassified roots and reports ownership
unsupported. It rejects marked owned roots, including when a marker appears
after that provider was constructed.
Owned providers also apply their ownership checks to unscoped orphan cleanup.

Version 2 ownership pins a separate private supervisor lock by device, inode,
and UID. `new_owned` retains its exclusive flock through clones, live instances,
monitor tasks, and in-flight IO. Another provider/process for the same root is
rejected until the supervisor lifetime ends. The lock is close-on-exec so stale
VM children cannot prevent restart takeover; their cgroups still require scoped
cleanup before lease release. Physical owner guards remain shared, allowing
different VMs to provision concurrently. Validation never creates metadata or
converts/clones the supervisor flock into a physical operation lock.

Version 1 markers and partial supervisor metadata fail closed without automatic
adoption. Use a fresh managed root for this profile. Existing live-instance
monitor references can retain ownership after facade destruction; reuse the
provider clone rather than constructing a second supervisor. These locks govern
cooperating current-version providers, not a malicious same-UID administrator
or an older unowned binary. Deployment must stop old supervisors and preserve
the database/version rollback gate before activating managed ownership.

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
Protocol version 11 rejects version 10 initializers before execution; rebuild
the initializer and cached guest roots together. Initialization defaults to
`None`. Only the exact checked frozen legacy state declaration may request
`BuiltinStateSQLite`. Historical scalar state labels use an explicit compatibility
conversion; scratch volumes never request database initialization.

Built-in initialization reopens the mounted filesystem without following links
and verifies its device and kernel mount identity. An isolated initializer child
pins that directory as its cwd before exec, rechecks its identity, and opens
relative `state.db` with SQLite `NOFOLLOW`. New databases use WAL and FULL;
existing schemas, bytes and journal modes are validated and preserved. Linked,
invalid or orphan database evidence fails closed. Writable roots and built-in
files retain the existing guest UID/GID 10001. Ordinary named SQLite data has
no built-in database; its workload owns initialization. This protocol slice does
not activate app composition or durable plural attachments. Native purpose
enforcement requires a fresh initializer and remains a separate verification.

## Native named-volume proof

Run `scripts/run-libkrun-named-volumes.sh` on the prepared KVM host. It builds
fresh current-protocol binaries and exports a disposable root from Ubuntu pinned at
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
