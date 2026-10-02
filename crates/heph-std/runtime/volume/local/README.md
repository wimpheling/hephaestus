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

# When

Initialize the root during host composition and provision registered resources
from a trusted worker. The legacy `resolve_instance_state` path uses the same
safe creation/recovery behavior and preserves an already reserved UUID and
capacity on retry. Runtime attachment still supports its legacy single state
volume; standalone attachment remains blocked pending named binding integration.
The local tests include native ext4 recovery and require Linux with e2fsprogs.
