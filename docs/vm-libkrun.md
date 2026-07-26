# libkrun backend

`vm-libkrun` is the Fedora/Linux implementation of the provider-neutral VM
runtime. It runs one dedicated worker process and one unprivileged `passt`
process per active networked VM.

## Host contract

Runtime processes execute as the dedicated, non-root `forge` account. They
never invoke privilege-escalation tools, create TAP devices, change routes, or
modify firewall rules.

One-time host provisioning must:

- install stable libkrun 1.x, matching libkrunfw, and `passt`;
- grant `forge` read/write access to `/dev/kvm`;
- create image, disk, mount, and runtime roots owned or readable as appropriate
  by `forge`;
- create a mode-0700 runtime root owned by `forge`;
- delegate a cgroup-v2 subtree with `cpu`, `memory`, `pids`, and `io`
  controllers to the `forge` service.

The backend dynamically opens `libkrun.so.1`. The configured API surface
requires libkrun 1.18 or newer within the stable 1.x series. Dynamic loading
keeps ordinary builds and unit tests independent of host library installation
while making a missing or incompatible runtime a typed provisioning error.

libkrun must include its block and network features. libkrunfw supplies the
guest kernel and firmware payload through libkrun's normal dynamic dependency.

## Process and cleanup model

Provisioning creates a private runtime directory, creates and configures a
per-VM cgroup, starts the worker, places it in the cgroup, connects framed IPC,
and asks the worker to load libkrun/libkrunfw. The guest is not booted until
`start`.

The worker owns libkrun, passt, virtio-fs devices, block disks, the guest
control socket, and their file descriptors. Children inherit the worker's
cgroup. Final cleanup uses `cgroup.kill`, waits for the cgroup to empty, removes
the runtime directory, removes the cgroup, and releases the VM identifier.

`passt` runs in foreground, one-off mode under the configured service UID/GID.
It supplies DHCP, DNS, TCP, and UDP egress. Ingress is limited to explicitly
declared TCP forwards on `127.0.0.1`; requested port zero is resolved before
passt starts and reported through `VmEvent::Started`.

## Configuration

`LibkrunConfig` requires:

- runtime, image, disk, and mount roots;
- the dedicated worker executable;
- the delegated cgroup-v2 root;
- expected service UID/GID;
- paths for passt and `/dev/kvm`;
- the libkrun shared-object name or path;
- startup/readiness deadlines and per-VM cgroup/disk/wall-clock limits.

All input paths are canonicalized and checked against their configured roots
before worker creation. The provider accepts directory roots and explicitly
typed raw root disks, and only explicitly typed raw additional disks. Mount
tags and disk IDs must be unique.

## Guest image

Approved images provide `/usr/libexec/hephaestus/heph-init`. It connects to the
host over virtio-vsock port 19,000 and speaks the versioned protocol exported
by `vm_libkrun::protocol`. The host sends start, cancellation, and health
messages; the guest sends readiness, logs, metrics, health responses, and one
final exit report.

## Validation

Run the hardware-independent suite with:

```sh
cargo test -p vm-libkrun
```

Real boot tests are disabled unless
`HEPHAESTUS_LIBKRUN_INTEGRATION=1`. The integration environment must also
provide:

- `HEPHAESTUS_LIBKRUN_RUNTIME_ROOT`
- `HEPHAESTUS_LIBKRUN_IMAGE_ROOT`
- `HEPHAESTUS_LIBKRUN_ROOTFS`
- `HEPHAESTUS_LIBKRUN_DISK_ROOT`
- `HEPHAESTUS_LIBKRUN_SQLITE_DISK`
- `HEPHAESTUS_LIBKRUN_MOUNT_ROOT`
- `HEPHAESTUS_LIBKRUN_REPOSITORY`
- `HEPHAESTUS_LIBKRUN_WORKSPACE`
- `HEPHAESTUS_LIBKRUN_CGROUP_ROOT`

The approved integration image must contain
`/usr/libexec/hephaestus/integration-check`, which reports `sqlite=ok`,
`dns=ok`, `tcp=ok`, and `udp=ok` after exercising those facilities.
