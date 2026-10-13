# Purpose

`volume-postgres` persists private-volume resources, provider intent, provisioning
progress, and the existing exclusive run leases on the canonical volume table.

# Responsibilities

The actor resource port uses authenticated identity and canonical live policy
checks, plus committed decision audit. Owner-project `can_manage` is required
for registration and replay. Inspection checks exact resource `can_read` before
reading a row; listing filters exact readable resources before paging. Denials
are committed to the audit separately from rejected effects. Public inspection
contains no host paths, host IDs, or encryption-key references.

Trusted workers reserve provider handles once. Immutable registration compares
ID, project, capacity, and filesystem UUID on replay. Database guards preserve
reserved UUID/capacity/host/path, and progress uses compare-and-set against an
independent provisioning generation. No implicit consumer grants are created.
The legacy resolver returns assigned intent rather than replacing it with fresh
retry options. Prior standalone rows with no UUID or handles receive a UUID once
in the additive migration; existing backing metadata remains unchanged.

Run lease foreign keys, uniqueness, and scalar execution evidence remain intact.
Standalone acquire fails until the later named runtime binding integration.
No SQL-provider abstraction or filesystem handles cross the metadata port.

The staged exact-run extension loads the immutable run revision, validates all
selected declarations and live typed grant issuer authority, then acquires the
whole set in one transaction. Locks follow consumer, run, global volume ID, and
lease order; logical slots determine returned order. Every lease and the exact
selection snapshot are durable before host IO. Retry compares the complete set;
closure prevents any later acquisition. Inspection loads all held leases before
checking host scope, including recovery-only historical evidence.

# When

Use `VolumeResourceRepository` with an actor pool for registration and safe
metadata discovery. Inject a separate trusted worker repository into the local
provider. The composition root owns migration timing. The ignored provisioning
integration test and the 0102-to-0103 upgrade fixture require a disposable
PostgreSQL database; native backing evidence also requires e2fsprogs.

Plural adapter tests currently apply private migration 108 to isolated databases.
Named positive cases explicitly simulate its runtime-support predicate and prove
database semantics only. Public migration 107 and normal application dispatch
remain unchanged; publishing 108 requires the legacy runtime cleanup crossover.

The worker-only `VolumeRootHistoryRepository` reads distinct immutable
`(root_path, owner_namespace)` pairs for the exact configured host across all
projects. Identical pairs coalesce; two pairs conflict. One different root is a
redirect conflict. Retirement, readiness, actor permissions and timestamps do
not filter history. This read creates no namespace, resource, marker or receipt.
A negative result is a query observation, never authority to initialize a root.

## Explicit scalar schema compatibility

`PostgresVolumeMetadataRepository::new` retains the public107 scalar queries.
Reviewed upgraded composition explicitly selects
`with_scalar_schema(ScalarMetadataSchema::Profiled)` after installing the private
profile108 and scalar compatibility fragments. Schema presence does not select
that mode and the mode grants no source or provider authority.

The profiled path accepts only positive fresh `legacy_scalar` Runs and exact
singleton `LegacyOrigin` leases. Acquisition commits the Run's exact volume,
lease and fence together while its VM remains unbound. Recovery permanently
closes acquisition and keeps the lease held until the trusted scalar provider
has completed its existing detach/cleanup boundary. Release commits the same
fence's Ready restoration atomically; duplicate replay never updates a reused
resource. Historical, owned canonical, named and update-only evidence stays
held by this scalar path. Existing canonical complete-set cleanup remains
separate and still requires its exact receipts.

A released legacy tuple records baseline scalar lifecycle semantics. It proves
neither physical absence nor an owned recipe's retention/deletion outcome.
Run binding/transition/restart integration and update callback qualification
remain separate tasks; this adapter does not activate default application
execution on the private schema.

The optional global scalar history lookup is worker-only and uses one read-only
repeatable-read snapshot. It returns one original held, recovery-fenced or released
lease, or genuine global zero. Foreign/update-only/multiple contradictory history
fails closed. Released lookup never requires or resets the current newer volume
generation. It is a metadata observation, not physical detachment proof.

## Original owned provisioning discovery

`discover_owned_provisioning` is a trusted worker readonly lookup using the exact
110 registration and configured physical host, root path and owner namespace.
The checked expectation derives the versioned first operation UUID from the
immutable creation operation; first birth uses expected generation zero and
operation generation one. It never selects a later CAS or creates an operation.

`Unadmitted` requires the original sealed birth, generation zero, reserved
metadata and genuinely zero global 111 purpose/operation/observation history. It
is not filesystem absence or admission permission. `Original` preserves the
original receipt, request, event, purpose, operation and current worker progress.
Interrupted or uncertain progress remains uncertain; Ready is database history,
not a replacement for host journal, inode or filesystem validation. Multiple,
foreign, retired and contradictory histories deny. The API grants no format or
claim authority and never reconstructs an authenticated identity.

## Retaining a ready owned volume

A current project and volume manager can close attachments, let the known run
owners drain their workloads, verify the original disk without writes, and retain
its data. Removing an installation does not borrow the original installer's
identity or require continued access to its old Source.

The service keeps data held when a consumer, lease, owner, original creation, or
physical disk cannot be checked. Actual journal custody stays owned through the
worker fact and manager's commit, including caller cancellation. Retention leaves
attachments closed; a new consumer needs fresh authorization and an explicit
reopening receipt. The current reopening path requires an originally completed
installation. An unfinished but physically ready birth retains its distinct
positive fact and remains held for reuse.

This opt-in backend path does not initialize roots, format disks, rewrite original
installation receipts, reset lease generations, enable named dispatch, or provide
physical VM shutdown evidence from database status. The checked scope pins come
from the configured provider and remain separate from volume-root ownership.

## Qualified Invocation lease/progress metadata

The explicit119 configuration requires the unpublished qualified119 schema
after113 through117. Default composition does not enable Invocation leases or
progress. Each operation compares the checked server configuration with the
protected producer and current full binding/grant set; configuration labels
do not prove physical VM or volume ownership. Acquisition freezes the whole
set, including zero, and exact own-fence replay never resets generations.

This slice supports queued admission through Provisioning and immutable held
inventory after execution rights are lost. Starting/Running, sessions, mounts
and actual VM execution remain unsupported. Cleanup cannot clear held fences
without the existing canonical provider and full-set completion receipts.
The global named-dispatch helper stays false.
