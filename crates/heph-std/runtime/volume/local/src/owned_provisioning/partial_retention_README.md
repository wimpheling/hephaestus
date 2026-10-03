# Partial birth custody

`LocalVolumeStore::observe_owned_partial` performs readonly observation with an
explicit server-supplied worker port and the existing original sealed claim.
It returns `OwnedPartialBirthCustody`, a concrete guard with no public constructor,
`Clone`, serialization, lock flag, or exposed descriptor. It owns the actual
nonblocking per-volume flock and pinned root, namespace and backing descriptors.

The service must hold this guard through worker recording and the eventual actor
transaction that permanently fences the birth and advances the recipe ledger.
Call `revalidate` before those boundaries. The receipt and checked observation
remain comparison data after the guard is dropped. Database observation CAS and
live manager checks are separate mandatory checks; this API cannot replace them.

Only positively recorded partial birth phases are accepted: empty claimed inode,
allocated never-format inode (including the exact allocation completion gap), or
incomplete first-format intent. Missing inode, conflicting phases, links,
substituted root/namespace/backing, and unsupported lengths remain held.
Readonly probing uses the existing UUID/geometry and `e2fsck -f -n` path under the
held flock. A clean Ready commit gap is refused and uses ordinary readonly Ready
reconciliation. Known SQL Ready is always refused, including later runtime dirtiness.
A timed-out child keeps its inherited flock until exit; no timeout proves absence.

This API creates, allocates, formats, publishes and records nothing. It performs
no actor admission and no first-format resumption. The worker adapter must reject
permanent retirement and compare exact current claim/head/purpose. The future
owning engine must persist and validate the extra provider-operation mapping
against the authoritative preceding Create claim before original invocation.

Tests use actual host files/flocks/mkfs/fsck with memory metadata fixtures. They
prove native custody only, not PostgreSQL receipt/fence transactions or recipe
completion. The private 0112 SQL proposal is not published or integrated here.
