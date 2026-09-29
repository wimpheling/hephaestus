# Purpose

`secret-runtime` materializes approved raw secret values into an ephemeral host
directory and exposes that directory as a read-only guest mount. It also
provides the cleanup and reconciliation operations that keep abandoned mounts
from surviving a worker or host restart.

# Responsibilities

The filesystem provider validates the dedicated root and, when configured,
requires `tmpfs` or `ramfs`. Materialization bounds file count and aggregate
size, rejects duplicate slots and unsafe paths, writes files mode `0400`, and
can include the short-lived runtime credential. Destruction requires confirmed
guest cleanup, while orphan reconciliation accepts only opaque UUID directory
names and removes non-live entries.

# When

Use `materialize` for raw files or `materialize_with_authority` when the guest
also needs broker or lease authentication. Install `FilesystemSecretMountProvider`
through the core run manager, persist its opaque directory identity before guest
attachment, and call `destroy_confirmed` after guest destruction.
