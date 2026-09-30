# Purpose

`forge-postgres` is the durable Forge adapter for projects, repositories, Git
receives, build triggers, revision inspection, run requests, and committed
outbox records. It gives core Forge services one transactionally consistent
view of metadata and accepted source changes.

# Responsibilities

`PgForgeRepository` creates and loads project and repository metadata, accepts
human or runtime receives, inspects exact commits and repository configuration,
persists build and instance triggers, and emits run requests. Receive methods
use the injected authorization boundary and storage adapter, preserve runtime
session provenance, and replay an existing receive ID without changing its
repository or source identity.

The adapter also resolves repository image configuration and UI manifests into
the durable projections consumed by build and release workflows. Its outbox
methods expose committed messages for the NATS publisher, with SQLx rows
converted into the provider-neutral Forge types at this boundary.

# When

Create `PgForgeRepository::new` with a PostgreSQL pool and `GitStorage`, add an
authorizer when the deployment requires it, and inject the repository into
Forge receive and project services. Call `accept_receive` or
`accept_runtime_receive` after the Git transport has validated the update.
