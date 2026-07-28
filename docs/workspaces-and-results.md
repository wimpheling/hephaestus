# Agent workspaces and controlled results

Phase 4 turns the exact commit accepted by the forge into a reviewable agent
result without placing a repository write credential in the guest.

## Workspace lifecycle

For each workspace-enabled run, `workspace-local` resolves the repository only
from its opaque repository UUID and reads the exact commit recorded on the run
request. It materializes two independent trees:

- `/workspace/repo` is the exact input tree and is mounted read-only;
- `/workspace/work` starts as a clean copy and is the only repository tree the
  guest may modify.

The canonical bare repository is never mounted. Submodules are not initialized;
`.gitmodules` remains ordinary source data.

The guest sends `FinalizeResult { message }` over the existing `heph-init`
vsock protocol. This is a one-way declaration that workspace mutation is
finished, not an acknowledgement contract. The orchestrator destroys and reaps
the VM before the host fsyncs and atomically renames the active workspace into
the sealed directory.

## Safe import and publication

Only the sealed path is imported. The importer uses `symlink_metadata`, never
follows workspace symlinks, rejects `.git`, absolute or traversing symlink
targets, non-UTF-8 paths, device nodes, FIFOs, sockets, and configured file,
entry, aggregate-byte, and patch limits. It constructs blobs and trees through
trusted Git plumbing and never consumes guest-controlled Git metadata or runs
hooks.

The host creates one commit whose parent is the exact input commit and
compare-and-swaps:

```text
refs/heads/hephaestus/<agent-id>/<run-id>
```

PostgreSQL records the input commit and tree, materialization hash, canonical
host and guest mount paths, result tree and commit, fixed result ref, and state
transitions. A prepared result is durable before ref publication; recovery can
repeat the CAS operation and complete the database state without producing a
second commit or moving an existing result ref.

## Durable outputs

The full imported-tree manifest, binary Git patch, captured VM logs, and final
exit event are stored under the configured artifact root with SHA-256, byte
size, storage key, and provenance. Files declared by
`results.declared_files` are retained separately with their path and Git mode.
Logs also remain durable run events and the final exit result remains on the
run record.

After successful publication, only the UUID-owned sealed workspace is removed.
Artifacts and canonical Git objects remain durable. Cancellation and failures
remove only the corresponding owned active or sealed materialization.
