# Purpose

`capability-audit-postgres` stores and inspects the redacted evidence emitted by
runtime capability checks. It connects the capability audit port to the
authorization database so operators can review one run without receiving
request payloads or secret values.

# Responsibilities

Workers append capability audit events in a transaction under the trusted
worker role. The `append_in_transaction` helper lets a capability adapter
commit its controlled mutation and the matching use outcome together. User
inspection starts an actor transaction, checks `can_read` on the exact run, and
returns the bounded cursor page through the application role.

# When

Construct `PostgresCapabilityAuditRepository` with the shared pool and inject
it into the worker and inspection paths. Append decision evidence immediately
after evaluation, use evidence with the operation outcome, and call
`list_for_run` only for a caller whose run permission has been established.
