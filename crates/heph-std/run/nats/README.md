# Purpose

`run-nats` transports durable start and cancellation commands through NATS
JetStream and invokes the provider-neutral run orchestrator. It keeps command
delivery responsive while long-running VM work proceeds and preserves broker
redelivery when processing fails.

# Responsibilities

The topology creates a file-backed work-queue stream for versioned Hephaestus,
Forge, and control subjects. `NatsCommandHandler` limits concurrent command
processing, decodes only the expected command for each subject, sends progress
acknowledgements for long starts, and double-acknowledges after orchestration
success. Failures remain unacknowledged for redelivery, while the database and
run orchestrator provide idempotency and authoritative lifecycle state.

# When

Create the durable topology during daemon startup and serve it with an
orchestrator:

```rust
let consumer = ensure_jetstream_topology(&context).await?;
NatsCommandHandler::new(orchestrator).serve(&consumer).await?;
```

The caller receives a durable command consumer; command success means the
orchestrator accepted the operation, not that a guest has already completed.
