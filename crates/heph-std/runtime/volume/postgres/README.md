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

# When

Use `VolumeResourceRepository` with an actor pool for registration and safe
metadata discovery. Inject a separate trusted worker repository into the local
provider. The composition root owns migration timing. The ignored provisioning
integration test and the 0102-to-0103 upgrade fixture require a disposable
PostgreSQL database; native backing evidence also requires e2fsprogs.
