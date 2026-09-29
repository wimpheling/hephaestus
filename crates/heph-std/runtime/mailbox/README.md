# Mailbox providers

The mailbox providers make core mailbox events durable and connect them to the
run workflow. PostgreSQL accepts the bounded envelope and opaque body once,
creates the delivery and outbox state, and remains authoritative for every
transition. The JetStream worker publishes and applies identifier-only wake,
dispatch, retry, cancellation, and recovery commands.

An eligible dispatch rechecks the active mailbox, instance gate, revision,
release, attachment, and stateful-run constraint while holding an
instance-scoped advisory lock. It then creates one run and one delivery
attempt. After the run is cleaned, the completion observer settles the delivery
from durable run facts; retry and dead-letter actions remain explicit operator
transitions.

The body never travels through JetStream or mutable command payloads. The run
runtime reads it only after the exact attempt has been authorized and binds it
to the sealed guest control tree.
