# Event contracts

The event branch defines the durable application-event boundary used after an
idempotent mutation commits. [`application/`](application/) supplies a receipt
reader for locating the committed event and an outbox port for publishing safe
event projections to downstream consumers.

The receipt carries the event identity, scope cursor, aggregate version, and
exact mutation lookup identity. The outbox projection contains safe state and
references suitable for publication while keeping persistence and broker work
in concrete adapters. A mutation handler can therefore return a durable receipt
only after its state and outbox write share the same commit.

Consumers should use the scope cursor and aggregate version to apply events in
order and should treat retry and dead-letter transitions as durable state. The
event boundary keeps plaintext credentials and provider-specific rows out of
product event payloads.
