# VM runtime contract

This document defines the contracts shared by the Hephaestus core, VM
providers, provider workers, and approved guest images. Provider-specific
mechanisms do not belong in `vm-trait`.

## Lifecycle

A VM is one-shot and moves through the following lifecycle:

```text
provisioned -> running -> stopping -> exited -> destroyed
      |           |                       |
      +-----------+-----------------------+-> destroyed
```

- `provision` allocates resources but does not boot the guest.
- `start` is idempotent. Concurrent callers share one startup attempt and all
  resolve successfully once that attempt succeeds.
- A successfully exited VM cannot be restarted.
- `stop` is idempotent. Calling it for a provisioned, stopped, or exited VM
  returns success. Graceful stop sends guest cancellation and force-stops the
  VM after the supplied timeout.
- `wait` supports any number of concurrent or later callers. The provider
  caches one final `VmExit` and returns it to every caller.
- `destroy` is idempotent. It force-terminates a running VM, reaps its worker
  and children, detaches mounts, and removes provider-owned runtime files.
- Destroying a VM before it has started causes present and future `wait` calls
  to return `VmError::Destroyed`.
- Destroying an exited VM does not discard its cached `VmExit`.
- Dropping the last `VmInstance` handle is not a cleanup operation. Callers
  must invoke `destroy`.

## Guest bootstrap and control

Every approved root filesystem contains `heph-init`. The provider boots
`heph-init`; it does not execute `GuestCommand` directly.

For the local microVM provider, `heph-init` and the provider worker communicate
over AF_VSOCK. The guest connects to host port 19,000. The protocol uses a
big-endian `u32` length followed by a CBOR payload. The initial handshake and
start command carry protocol version `1`, and peers reject unsupported
versions. Frames are limited to 16 MiB, individual log chunks to 64 KiB,
metric names and label components to 256 bytes, and metrics to 64 labels.
Unknown wire variants are rejected for protocol version 1; Rust's
`#[non_exhaustive]` API marker does not provide wire compatibility.

The bootstrap exchange is:

1. The guest sends `Hello { version }`.
2. The worker sends `Start { command, mounts }`.
3. The guest validates the command and sends `Ready`.
4. The guest sends zero or more ordered `Log { stream, bytes }` messages.
5. The guest sends exactly one `Exited { code, signal }` message.

`Start.command.program` is an absolute guest path and is executed directly,
without a shell. Its arguments do not include `argv[0]`. The command does not
inherit the host environment; `Start.command.env` supplies its environment.
`working_dir`, when present, must be absolute and accessible to the guest
account.

A graceful stop sends `Cancel { timeout_ms }` to `heph-init`. The worker waits
at most that duration before force-stopping the VM. A forced stop terminates
the VM without requiring a guest acknowledgement. Disconnecting the control
channel does not itself define the guest exit status; the worker must obtain a
status or report an unknown `VmExit`.

Log bytes are not required to be UTF-8. Ordering is preserved within each
stream; interleaving between stdout and stderr reflects the order in which the
worker receives frames. A known normal exit sets only `code`, a known
signal-based exit sets only `signal`, and an unknown status leaves both absent.
Both fields must never be present simultaneously.

The provider translates the exchange into `VmEvent` values. `Started` means
the VM is running and includes resolved ingress assignments. `Ready` means
`heph-init` accepted the command. Event order is:

```text
Started -> optional Ready -> zero or more Log/Metric -> exactly one Exited
```

The broadcast event stream is live telemetry and may lose logs under
backpressure. It is never the source of truth for completion. `wait` and its
cached final exit status provide the reliable completion path.

Ingress forwarding in `vm-libkrun` initially supports TCP bound to the IPv4
loopback address. A requested host port of zero asks the provider to allocate
a port. The `Started` event contains the effective rules with every allocated
host port resolved to a nonzero value.

## Parent and worker

The Hephaestus parent starts one worker process per local VM. A private,
length-framed Unix stream socket in the mode-0700 runtime directory carries
versioned CBOR messages with request identifiers.

Parent-to-worker requests are `Configure`, `Start`, `Cancel`, `Health`, and
`Destroy`.
Worker-to-parent messages are acknowledgements, typed failures, lifecycle
events, and `CleanupComplete`. Each request carries a monotonically increasing
`u64` request identifier; its acknowledgement or failure echoes that
identifier. Unsolicited lifecycle events do not carry a request identifier and
are ordered on the worker connection.

The worker exclusively owns:

- the hypervisor context and its file descriptors;
- guest-control and networking processes and file descriptors;
- the VM mount namespace and provider runtime directory.

The parent owns:

- the public `VmInstance` lifecycle state and cached exit status;
- worker supervision and reaping;
- cgroup creation and assignment;
- event forwarding to the live broadcast channel and durable telemetry system;
- fallback cleanup when the worker exits unexpectedly.

The parent does not report `destroy` success until the worker and its children
have been reaped and provider-owned files and mounts have been removed.

## Images, disks, and mounts

Disk-backed roots and additional disks always declare a `DiskFormat`. A
provider must reject unsupported formats and must never inspect mutable guest
data to guess a format.

Paths supplied in a `VmSpec` are caller-owned:

- providers never delete a supplied root, disk, or mount path;
- `read_only` inputs must not be mutated;
- writable disk images are allocated, sized, and formatted by the Hephaestus
  storage layer before provisioning;
- providers may create private overlays or uploaded copies and must remove
  those provider-owned resources during `destroy`.

Before provisioning, the core canonicalizes root, disk, and mount paths,
rejects symlink escapes, and validates mounts against its configured allowlist.
Providers repeat the containment and file-type checks as defense in depth.

`heph-init` performs the privileged bootstrap operations needed to mount guest
filesystems. The initial implementation executes `GuestCommand` with the
bootstrap process's image-defined identity; guest identity selection is not
yet part of the provider-neutral trait. Mount contents must be staged with
ownership and permissions appropriate for the approved image's command
identity.

`GuestCommand::working_dir`, when present, is an absolute path inside the guest.
It must exist after mounts are attached and be accessible to the guest account.

## Deferred APIs

Snapshots, pause/resume, secrets, authorization policy, network policy,
provider capability discovery, and recovery of instances after a parent
restart remain outside the initial contract.
