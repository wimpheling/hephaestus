# Purpose

`runtime-authority-postgres` persists immutable authorization snapshots and
hash-only runtime sessions. It gives trusted workers and gateway admission code
the database side of exact runtime authority issuance.

# Responsibilities

`PgRuntimeSessionRepository::resolve_snapshot` verifies that a run is in the
correct provisioning state, selects its exact revision bindings, and builds the
normalized snapshot. `live_authorized` rechecks every snapshotted operation
against current authorization immediately before provisioning. The gateway
issuer performs the equivalent revision and handler-contract checks for guest
or host-mediated gateway sessions, while persisting no plaintext bearer.

# When

Create `PgRuntimeSessionRepository` over a worker-role pool while preparing an
agent run, then pass its snapshot and identity to the core runtime issuer. Use
`PgGatewayRuntimeAuthorityIssuer` for accepted gateway invocations and choose
the guest or host-mediated issuance method according to the handler contract.
