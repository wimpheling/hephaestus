# Purpose

`release-artifact-store` imports a sealed build-output directory into a local
immutable store. It gives release persistence a deterministic manifest of
release-relative paths, content hashes, modes, sizes, and opaque storage keys.

# Responsibilities

The store accepts only safe ordinary files beneath an administrator-owned root,
rejects symlinks, hard links, special files, unsafe modes, traversal, source
mutation, and configured file or byte limits, and hashes bytes while copying.
Canonical objects are write-once and repeated imports can derive stable keys
from an operation identity. Reads use no-follow file access and verify length
and hash before returning bytes, so a path swap or corrupted object cannot
become release content.

# When

Use this adapter after a build has sealed its output and before persisting the
release manifest:

```rust
let store = LocalArtifactStore::new(store_root)?;
let artifacts = store.import_for(operation_id, sealed_output)?;
```

Persist the returned manifest with the release and resolve objects only by the
opaque storage keys and expected hashes it contains.
