# Purpose

`review-postgres` persists review control requests, approval proposals, run
events, and the command outbox. It is the transactional adapter behind browser
review controls and the provider ports consumed by the NATS and Git adapters.

# Responsibilities

Each command loads and matches its durable control row, sets the database actor
context, checks current authorization, and commits state changes with the
corresponding event or outbox row. Cancel, retry, and reject operations become
durable outcomes; approval is prepared in one transaction, published through
the trusted Git adapter, and finalized only when the recorded input and result
commits still match. A stale or unauthorized command is denied or conflicted
without mutating the proposal. Repository location is resolved through
canonical forge storage rather than a caller-supplied filesystem path.

# When

Compose the repository with a PostgreSQL pool, authorization provider, and Git
storage, then pass it to the review service:

```rust
let repository = PostgresReviewRepository::new(pool);
```

The service returns a durable control outcome and leaves committed outbox work
for `review-nats` to publish.
