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

## Explicit routing boundary

`NatsCommandHandler::new(Arc<RunOrchestrator>)` retains direct dispatch. The
additive `new_routed(Arc<dyn RunCommandExecutor>)` delegates to an app-provided
checked executor. NATS neither selects profiles nor queries RunPG; JSON fields,
slot counts and schema presence grant no routing authority. Both constructors
retain 64 concurrent deliveries, ten-second Start progress acknowledgements,
confirmed final acknowledgements only after success, and error redelivery.
Cancellation can execute while another Start is blocked.

The explicitly ignored `command_executor` target uses a dedicated disposable
JetStream account/server configured by `HEPHAESTUS_NATS_COMMAND_EXECUTOR_TEST_URL`.
It exercises actual broker success acknowledgement, unacknowledged error
redelivery and cancellation/progress while a controlled executor blocks Start.
This is transport evidence, not a VM, database admission or profile authority
proof. The application router and managed startup remain separate work.
