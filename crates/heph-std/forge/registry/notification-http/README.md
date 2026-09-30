# Purpose

`registry-notification-http` is the private HTTP ingestion edge for Zot
CloudEvents. It converts authenticated registry callbacks into bounded,
validated observations that the registry control plane can reconcile later.

# Responsibilities

The endpoint enforces a body limit, verifies the configured callback
credential, parses the CloudEvent and Zot payload, and inserts the observation
through an idempotent inbox port. Replayed events receive an accepted duplicate
response, while forged, malformed, oversized, or unavailable requests receive
safe retry-oriented errors. The adapter stores an observation before returning
success so best-effort Zot callbacks cannot be mistaken for an authoritative
publication decision.

# When

Configure Zot's event sink with the private route and a callback credential,
then compose the service with the PostgreSQL inbox:

```rust
let router = RegistryNotificationHttpService::new(callback_credential, inbox).router();
```

The caller receives an accepted or duplicate disposition; reconciliation must
still inspect the registry by exact digest before marking a publication usable.
