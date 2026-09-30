# Mailbox runtime contracts

The mailbox branch defines how an external event becomes one durable,
idempotent delivery attempt for an agent instance. It sits between an ingress
adapter, the run dispatcher, and the guest runtime: ingress stores bounded
metadata and opaque body references, dispatch assigns an ordered attempt, and
runtime preparation places the accepted body in the guest control mount.

[`domain/`](domain/) contains the provider-neutral envelope, identity, routing,
and delivery state contracts. PostgreSQL, NATS, and VM adapters can implement
their own persistence and transport while preserving the same state machine.
The body remains opaque to control-plane code; only the exact accepted attempt
is eligible for a run.

## Delivery shape

An ingress request is normalized into method, route, selected headers, bounded
content metadata, trace context, and a body integrity reference. A producer
identity and deduplication key make repeated ingress idempotent. The dispatcher
advances the event through eligible, leased, running, delivered, retryable, or
dead-lettered states and assigns a per-instance sequence when state ordering is
needed.

The runtime orchestrator consumes that accepted record only after the run's
release, revision, and authority have been resolved. This keeps arbitrary
broker payloads and untrusted headers from becoming guest input by accident.
