# Purpose

`event-postgres` persists the database side of durable application events. It
lets transports load the committed result of an idempotent mutation and lets a
worker publish product events only after their source transaction is durable.

# Responsibilities

`PostgresMutationReceiptReader` matches an occurrence to its actor, aggregate,
and primary scope before returning the event cursor and aggregate version.
`PostgresProductEventOutbox` reads unpublished release-owned records, publishes
them with a stable `Nats-Msg-Id`, and marks success, retryable failure, or dead
letter state in PostgreSQL. The topology helper creates the bounded release
event stream and its supported subjects.

# When

Construct the receipt reader with the event database pool for request replay.
Run `ReleaseOutboxPublisher::publish_pending` from a worker after calling
`ensure_release_jetstream_topology`; a successful JetStream acknowledgement is
the point at which the corresponding outbox row is marked published.
