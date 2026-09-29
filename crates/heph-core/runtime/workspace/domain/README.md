# Purpose

`workspace-domain` defines the provider-neutral lifecycle for source
workspaces, runtime-Git workspaces, and agent results. It binds each materialized
workspace to an exact run and commit, then carries sealed artifacts to the
controlled publication boundary.

# Responsibilities

Workspace requests record repository, ref, commit, and capability-scoped Git
operations. The lifecycle ports cover preparation, activation, finalization,
abandonment, and crash recovery; result ports retain pending, prepared, and
completed metadata and the VM evidence associated with the run. The contract
keeps source input immutable, rejects traversal and unapproved Git targets, and
lets only a trusted host importer publish a result ref. Runtime-Git uses a
dedicated loopback path and explicit capability worktree rather than inferring
write access from a legacy source mount.

# When

Use this crate when a run is being prepared or when a result is being sealed:

```rust
let prepared = workspace_manager.prepare(&run).await?;
```

Finalize the prepared workspace through the host adapter, persist its result
metadata, and invoke publication only after the run outcome and declared result
policy have been checked.
