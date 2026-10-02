# Sealed local volume birth

`LocalVolumeStore::with_owned_metadata` explicitly injects an actor admission
adapter, a distinct worker observation adapter, and the persistent volume-root
owner guard. A single PostgreSQL role pool cannot provide both authorities.
The same canonical volume row/provider and existing per-volume flock are used.

The trusted engine must compare the actual authoritative preceding Create claim
to the full original creation seal/correlation before calling this provider.
Checked labels, predicted IDs, matching UUIDs, metadata row ownership and a
canonical filename do not establish file ownership or recipe authorization.

The first actor admission commits before host IO. The private root marker binds
its UUID to root device/inode/UID and configured host; this is distinct from VM
root ownership and orchestration scope. Legacy independent files can coexist
with a prospective root marker but are never adopted. Missing markers beside
owned journals, copied markers and replaced roots fail without foreign writes.
Journal acquisition and every namespace/backing write use the owner's pinned
root descriptor; replacing the path cannot redirect lock creation into a foreign
directory. Path/marker validation surrounds acquisition.

The journal publishes durable complete namespace/birth evidence before returning
an empty claimed inode. Allocation intent precedes `set_len`; a fresh original
actor can sync an exact-capacity claimed allocation gap without resizing. A
zero-length claimed birth can resume allocation under the same positive
never-format evidence. Other lengths or contradictory filesystem bytes stay held.
Allocation completion, inode-preserving NOREPLACE publication and directory
fsync precede durable first-format intent. Once that intent exists, no retry can
format again. The actual formatter child inherits the same-volume flock through
stdin, retaining exclusion across parent future cancellation until actual exit.
Actor authority is rechecked after the worker format-intent record returns and
immediately before spawning mkfs. Revocation leaves that intent held without
formatting, even if actor authority is subsequently restored.

Readonly worker reconciliation opens only existing lock/journal/backing
descriptors. It performs no create, allocation, journal publication or first
format, even after creator revocation. Ready requires durable original format
intent, exact birth inode, clean UUID/4096-byte geometry and successful
`e2fsck -f -n`. Incomplete allocation/publication/format phases return held errors;
they do not claim Ready, absence, failed creation or permission to reformat.
An already committed Ready result is historical birth evidence; this path does
not run fsck against an attached runtime volume. Later writable journal replay
belongs to canonical run lease recovery.

The native tests use real installed `mkfs.ext4` and `e2fsck`, plus injected memory
metadata ports. They prove host behavior only. Ordered private 0110/0111 migration
tests and later composed PostgreSQL/host tests remain separate verification.
Production generic metadata exclusions must accompany coherent sealed-schema
publication; this module does not publish migrations or enable legacy startup.
