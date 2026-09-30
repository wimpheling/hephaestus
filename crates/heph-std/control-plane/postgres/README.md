# Purpose

`control-plane-postgres` is the PostgreSQL adapter for control-plane repository
metadata and launch workflows. It turns persisted project, release, build,
instance, repository, and run state into the typed values consumed by core
services.

# Responsibilities

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
