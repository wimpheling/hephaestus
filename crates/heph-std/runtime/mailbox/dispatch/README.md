# Purpose

`mailbox-dispatch` applies durable mailbox commands from JetStream and links a
claimed delivery attempt to the run orchestrator. It also observes run
resources and completion so the mailbox state follows the authoritative VM
lifecycle.

# Responsibilities

The handler accepts only the versioned mailbox subjects, decodes stable event
and operation identifiers, and asks the PostgreSQL dispatch port to apply each
compare-and-swap transition. A dispatch command can create one `StartRun`; the
orchestrator then owns VM preparation and cleanup. The completion observer
settles delivery after cleanup and the resource observer records the run's
pre-provision evidence. A command is acknowledged only after durable effects
and orchestration succeed, so redelivery remains safe and mailbox bodies,
capability bearers, and attempt state never enter NATS.

# When

Compose the command handler with the authoritative store and run orchestrator:

```rust
let handler = MailboxCommandHandler::new(store, orchestrator);
```

Give `NatsMailboxCommandHandler` to the JetStream consumer; successful handling
returns an acknowledged delivery, while any failure leaves the command
available for retry or recovery.
