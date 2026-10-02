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

# When

Use the actor resource port for authorized metadata operations. Use the trusted
provider ports only in host workers. Keep provider handles out of transports,
recipes, and resource inspection. Attachment authority differs from permission
to query an application's database.
