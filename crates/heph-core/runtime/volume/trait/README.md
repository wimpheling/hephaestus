# Purpose

`volume-trait` defines private-volume resource metadata and trusted provider,
provisioning, and attachment ports.

# Responsibilities

`VolumeResourceRepository` registers an immutable predicted ID, owning project,
filesystem UUID, and exact capacity after live audited authority. Inspection and
bounded retained discovery return `VolumeInspection`, which contains no host
handles or key references. Creating a resource grants no agent capability.
New local registration requires 16 MiB through 16 TiB, aligned to 4 KiB. The
legacy release declaration's one-byte minimum describes compatibility only.

`Volume` and `ProvisioningClaim` are trusted records containing provider handles.
Provisioning uses its own monotonic metadata fence and an exclusive host lock.
Unknown existing backing requires reconciliation; expiry authorizes no format.
`VolumeStore` retains the existing single state-volume run attachment contract.
Standalone and named runtime attachment require the later plural integration.
Existing leases retain their exclusive run/volume fence and recovery semantics.

The additive `RunVolumeMetadataRepository` and `RunVolumeStore` ports describe
zero to 32 exact revision selections and complete durable lease sets. Readonly
and writable attachments are both exclusive initially. Selected leases carry
checked declaration provenance; unmatched historical leases remain recovery
evidence and cannot authorize new acquisition. Storage checks do not replace
the caller's live source and execution authorization. Whole-set destruction and
release use the separate canonical run cleanup contract.

# When

Use the actor resource port for authorized metadata operations. Use the trusted
provider ports only in host workers. Keep provider handles out of transports,
recipes, and resource inspection. Attachment authority differs from permission
to query an application's database.

The plural metadata adapters are staged against private migration 108. Existing
application composition still uses the legacy path; named dispatch remains
unsupported until runtime and legacy cleanup integration are complete.

`VolumeMountGrantRepository` creates audited explicit immutable typed mount
grants and permanent revocations. Creation requires consumer management and
source grant plus attachment authority. A revoked revision/slot cannot be
silently regranted. This port neither activates revisions nor mounts volumes.

`VolumeRootHistoryRepository` lets trusted workers read all immutable owned
purpose history for one configured host, including retired births. It returns
no history, one exact expected root namespace, or a bounded conflict. Other
hosts may use the same absolute path independently. A no-history result grants
no bootstrap authority; normal startup must reopen an expected owner or require
an explicit prospective bootstrap decision.

The optional `scalar_lease_history` port observes global original Run history.
`NoHistory` is genuine global zero, and `HeldRecovering` comes from persisted
recovery state rather than expiry. Multiple, foreign or contradictory rows deny.
It grants no provider IO or lease-release authority; unsupported adapters fail
closed. A released original tuple does not describe a newer resource generation.

## Original owned provisioning discovery

Use original provisioning discovery to recover an interrupted installation's exact
volume operation from its sealed creation record and configured disk owner, then
resume recovery with that original operation.

`discover_owned_provisioning` is a trusted worker readonly lookup using the exact
sealed registration and configured physical host, root path and owner namespace.
The checked expectation derives the versioned first operation UUID from the
immutable creation operation; first birth uses expected generation zero and
operation generation one. It never selects a later CAS or creates an operation.

`Unadmitted` requires the original sealed birth, generation zero, reserved
metadata and genuinely zero global purpose/operation/observation history. It
is not filesystem absence or admission permission. `Original` preserves the
original receipt, request, event, purpose, operation and current worker progress.
Interrupted or uncertain progress remains uncertain; Ready is database history,
not a replacement for host journal, inode or filesystem validation. Multiple,
foreign, retired and contradictory histories deny. The API grants no format or
claim authority and never reconstructs an authenticated identity.

## First-consumer filesystem verification

`assert_owned_first_ready_verification` checks a trusted worker's exact original
owned operation before an owning provider verifies its Ready filesystem. It
requires positive protected creation correlation, unfinished first completion
and genuinely zero global runtime lease history. A dedicated durable guard
blocks every runtime lease until that same creation has immutable successful
completion. Missing, contradictory or inactive claims never open admission.

This readonly assertion grants no filesystem proof, format permission or actor
completion authority. Unsupported repositories deny. Completed stale calls
must use protected progress replay, not another filesystem check.

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
