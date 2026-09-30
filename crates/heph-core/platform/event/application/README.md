# Purpose

`event-application` defines ports for reading committed mutation receipts and
publishing durable product-event projections. It lets application services
return an exact event identity after an idempotent write and lets a worker drain
the committed outbox through a provider-neutral contract.

# Responsibilities

`MutationReceiptReader` matches occurrence, actor, aggregate, and primary scope
before returning the committed event cursor and aggregate version. The
`ProductEventOutbox` port models pending, published, retry-failed, and
dead-lettered projections. The projection fields are safe, bounded metadata;
the storage adapter owns the transaction and the broker adapter owns delivery.
This boundary prevents an event from being published before its mutation is
committed and gives consumers stable ordering evidence without exposing
provider rows or secret values.

# When

Use the receipt reader after an idempotent mutation or use the outbox port in a
publisher worker:

```rust
let receipt = reader.load(request_id, actor_id, "run", "run").await?;
```

Return or publish the receipt only from the committed transaction path, then
mark each outbox record according to the broker result.
