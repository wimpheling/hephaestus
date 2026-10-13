# Purpose

`control-plane-postgres` is the PostgreSQL adapter for control-plane repository
metadata and launch workflows. It turns persisted project, release, build,
instance, repository, and run state into the typed values consumed by core
services.

# Responsibilities

The existing scalar launch authorizer retains its legacy behavior. The separate
`authorize_canonical_source` helper requires a worker and exact original DB
actor/request correlation before auditing source use and attachment execution
or instance update permission. It also checks current active-user and consumer
gates. A trusted app wrapper first validates the owned live snapshot and session;
this helper grants no mount permission, new credential, dispatch support or
renewal. Intentional issued normal drain may check source rights after acquisition
closure; preissuance closure, cancellation and removal deny. No new dependency
between PostgreSQL adapters is needed.

The adapter keeps SQL queries in the control-plane boundary, uses the
authorization transaction helpers when a caller acts for a user, and streams
large artifact results through bounded storage paths. `connect_worker` selects
the worker role and `connect_app` selects the application role; contract checks
can verify that migrations and the permission dispatcher are installed before
serving traffic.

# When

Open a pool with `connect`, `connect_worker`, or `connect_app` at composition
time, then pass it to the relevant repository service. Use the worker pool for
launch, artifact, and lifecycle mutations, and the application pool for
authorized control-plane reads.
