# Purpose

`forge-nats` publishes committed Forge outbox records through NATS JetStream
and establishes the build and run subjects and consumers used by the worker
topology. It turns durable database events into retryable delivery without
making event publication part of the original receive transaction.

# Responsibilities

`ensure_forge_jetstream_topology` creates or verifies the stream and subjects,
while `ensure_build_consumer` installs the durable build consumer. The
`ForgeNatsOutboxPublisher` reads pending records from `ForgeOutboxStore`,
publishes each payload to its recorded subject, and marks success or bounded
failure so retries preserve the committed event.

Subject constants cover build requests and retries, verification, instance
runs, and run starts. Provider errors remain typed so the daemon can
distinguish topology, serialization, and publication failures.

# When

Call the topology helpers during Forge worker startup, then construct
`ForgeNatsOutboxPublisher` with the JetStream context and run
`publish_pending` against the PostgreSQL-backed outbox.
