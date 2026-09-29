# Purpose

`workspace-postgres` persists the metadata used by local source and result
workspaces. It records requests, exact input commits, preparing and active
paths, runtime-Git provenance, result states, artifacts, and lifecycle events.

# Responsibilities

The adapter translates workspace-domain ports into transactions that mark
materialization active or failed, persist result and artifact metadata, and
record cleanup or runtime-Git recovery events. Queries bind a run or command to
the release, instance, repository, capability snapshot, and exact commit so a
filesystem adapter cannot prepare a workspace from unrelated provenance. It
stores state and diagnostics needed to recover incomplete work without placing
filesystem paths or Git process execution in SQL callers.

# When

Construct the repository over the application pool and inject it into
`LocalWorkspaceManager`:

```rust
let metadata = PgWorkspaceMetadataRepository::new(pool);
```

The local adapter uses the returned metadata to prepare, finalize, abandon, and
recover host trees while callers receive provider-neutral workspace values.
