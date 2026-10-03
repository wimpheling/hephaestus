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
