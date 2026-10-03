# Purpose

`volume-local` creates and recovers a private ext4 backing file using the existing
canonical metadata repository and the configured single host.

# Responsibilities

`provision` accepts a stable resource ID. The metadata adapter reserves the host,
provider-generated `<volume-id>.raw` path, capacity, and UUID before backing IO.
The local adapter opens a private provider-owned root and direct children through
descriptors with no-follow flags, rejects symlinks and hard links, and holds an
exclusive per-volume OS lock before claiming a durable provisioning generation.
Only exclusive creation of an absent file permits allocation and formatting.
Allocation and its directory entry are synced before the durable format intent;
the formatted file is synced and proven before readiness is recorded.

Existing files are never resized or reformatted. Recovery requires the expected
ext4 UUID, exact file capacity, exact filesystem block geometry, and a complete
readonly `e2fsck -f -n` check. The configured `mkfs.ext4` directory must also
contain `e2fsck`. Unknown or partial backing stays uncertain with its bytes
retained. Encryption intent fails closed because this provider does not implement
it. These checks do not implement guest mounts or replace later mount resolution
checks against symlinks and protected runtime paths.

The staged exact-run attachment path verifies already provisioned ready backing
without formatting, resizing, or clearing filesystem flags. It checks the safe
root and file identity, UUID, capacity, and geometry. Dirty readonly backing is
rejected. A known ready writable filesystem may retain normal ext4 recovery
flags for guest journal replay under exclusive leases and confirmed old-writer
destruction; this differs from adopting an uncertain initial-format result,
which still requires the complete clean filesystem proof above. Host file
identity checks do not prove original backing birth ownership.

# When

Initialize the root during host composition and provision registered resources
from a trusted worker. The legacy `resolve_instance_state` path uses the same
safe creation/recovery behavior and preserves an already reserved UUID and
capacity on retry. Runtime attachment still supports its legacy single state
volume; standalone attachment remains blocked pending named binding integration.
The local tests include native ext4 recovery and require Linux with e2fsprogs.

`with_run_metadata` explicitly injects the staged complete-set metadata port.
Without it, plural calls fail before provider effects and cannot fall back to
the scalar resolver. Current application startup does not inject this port.
Metadata acquisition precedes backing inspection, so inspection failures retain
all durable leases for canonical run cleanup and recovery.

`VolumeRootOwner::open_existing` reopens a previously owned root using the
authoritative expected host and `VolumeRootNamespaceId`. It reads the existing
marker and validates the actual root and marker descriptors without creating or
repairing files. Missing or substituted roots and mismatching evidence fail held.
The expected namespace must come from trusted persisted configuration or
metadata; the current filesystem path is not a source of authority.
`VolumeRootOwner::initialize` remains the prospective initialization API.

## Bootstrapping a configured disk root

`VolumeRootOwner::bootstrap_empty_with_namespace` publishes an operator-chosen,
checked UUID only in an already existing, canonical and empty prospective root.
It creates no directory, chooses no UUID, adopts no raw file, and repairs no
existing or interrupted marker. Save the UUID in trusted durable configuration
before the explicit bootstrap; restart with `open_existing` and that same UUID.
An existing marker, pending record or other entry makes bootstrap fail held.

The new operation retains pinned descriptors, checks emptiness and linked-root
identity around durable no-replace publication, and uses a transient nonblocking
root-directory lock to serialize cooperating new bootstraps. The lock uses its
own open file description and is released before an owner is returned; it is not
a lifetime supervisor or exclusion of arbitrary same-UID writers. Existing
`initialize` and its unsealed-file behavior are unchanged. Interrupted or racing
evidence is retained for a separate recovery decision.

This filesystem API establishes neither database absence nor operator authority.
The owning composition must supply the configured namespace and independently
check all immutable root history; an empty history observation grants no
bootstrap permission. Disk UUIDs stay separate from VM ownership labels.
