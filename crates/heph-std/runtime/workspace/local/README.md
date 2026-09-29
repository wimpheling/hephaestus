# Purpose

`workspace-local` materializes exact-commit source workspaces and publishes
trusted Git results on the host. It implements the core workspace lifecycle for
ordinary read-only source input, runtime-Git capability worktrees, result
artifacts, and crash recovery.

# Responsibilities

The manager validates absolute non-overlapping roots and the configured Git
binary, resolves the request's repository and exact commit, records metadata,
and seals source and result trees with owner markers and bounded paths. Runtime
Git uses the stored capability and ref policy to create a dedicated worktree;
it never infers write access from a source mount. Finalization validates
declared files, hashes and stores artifacts, and updates a target ref only after
the trusted result provenance and expected input commit are checked. Recovery
removes only owned preparing, active, and sealed paths and marks their metadata
accordingly.

# When

Create the manager with PostgreSQL metadata and result ports, then install it
on the run orchestrator:

```rust
let manager = LocalWorkspaceManager::new(metadata, results, config)?;
manager.initialize()?;
```

The caller receives prepared read-only mounts, a published result, or a
redacted lifecycle error suitable for durable run recovery.
