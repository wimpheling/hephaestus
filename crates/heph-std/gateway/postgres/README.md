# Purpose

`gateway-postgres` is the PostgreSQL authority for repository-declared HTTP
gateways and long-lived service instances. It turns an immutable release
manifest into authorized routes, runtime contracts, service leases, and
durable invocation evidence for the gateway edge.

# Responsibilities

The installer parses and bounds the exact manifest before opening its
transaction, checks project management authority, and atomically persists the
declaration revision and routes. The adapter also persists mailbox publication
state, resolves runtime and service targets, fences service ownership through
leases, and stores bounded service logs and failures. It does not open the edge
listener or derive provider configuration.

# When

Use `PostgresGatewayInstaller` with a worker-authorized pool when installing a
release's gateway declaration. Use the management, ownership, target, log, and
runtime resolver ports from the same provider to prepare the edge and service
coordinator; hand active route state to `gateway-edge` for reconciliation.
