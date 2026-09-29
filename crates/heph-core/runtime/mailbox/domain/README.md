# Purpose

`mailbox-domain` defines the durable contract for delivering one bounded event
to an agent instance. It gives ingress and dispatch code the same envelope,
producer identity, deduplication key, body reference, and delivery lifecycle
without coupling them to PostgreSQL, NATS, or a VM implementation.

# Responsibilities

Envelope constructors validate method, route, selected headers, trace context,
content metadata, body size, and the SHA-256 body reference. The delivery state
model records ordered attempts and permits only deliberate transitions from
pending through leasing and execution to delivery, retry, or dead lettering.
These bounds keep control-plane metadata small and prevent header injection or
an unverified body from crossing into a guest. The body bytes remain an opaque
reference until an already-authorized runtime requests the exact attempt.

# When

Use this crate when an ingress adapter accepts an event or when a dispatcher
needs to validate and advance its durable delivery record:

```rust
let envelope = MailboxEnvelope::new(
    method,
    route,
    selected_headers,
    content,
    received_at,
    trace_context,
)?;
```

Persist the resulting envelope and its producer-scoped deduplication key before
asking the run workflow to deliver it.
