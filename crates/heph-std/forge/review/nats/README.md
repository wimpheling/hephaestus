# Purpose

`review-nats` transports committed review control commands through NATS
JetStream. It publishes durable outbox rows and handles delivered commands so
the review control plane can survive broker retries and process restarts.

# Responsibilities

`ReviewOutboxPublisher` claims pending command rows, serializes their durable
payloads, sets a row identity as the JetStream message ID, and marks a row
published only after acknowledgement. Failures remain recorded for retry.
`NatsControlHandler` accepts only the review control subject, decodes the
command, invokes the control service, and double-acknowledges only after the
service succeeds. Unsupported subjects, malformed payloads, processing errors,
and acknowledgement failures remain visible to redelivery and operations.

# When

Create the publisher and handler beside the daemon's JetStream connection:

```rust
let publisher = ReviewOutboxPublisher::new(context.clone(), outbox_store);
let handler = NatsControlHandler::new(control_service);
```

Run the publisher against committed rows and let the handler leave failed
deliveries unacknowledged so JetStream can redeliver them.
