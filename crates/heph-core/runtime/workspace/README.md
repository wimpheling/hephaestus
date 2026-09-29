# Workspace contracts

The workspace branch separates immutable run input from mutable output. Its
domain contracts cover exact-commit source materialization, optional runtime-Git
workspaces, result metadata, artifact collection, and the controlled import of
published refs.

[`domain/`](domain/) is consumed by run orchestration after authority and
release resolution. A normal run receives a source snapshot tied to one
repository commit. A runtime-Git run receives a capability-scoped worktree and
loopback bridge. Both workflows give the guest a bounded writable area and
record the metadata needed to reconcile it after a crash.

At completion, the host seals the result, validates declared paths and
artifacts, and asks the trusted repository adapter to publish an approved ref.
The guest cannot choose a repository, ref, or publication target by writing
arbitrary Git metadata. Recovery finds preparing or pending workspaces and
results and either resumes a safe transition or marks them failed.
