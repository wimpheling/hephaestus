# Purpose

`mailbox-postgres` is the durable authority for mailbox allocation, event
acceptance, delivery claims, operator controls, retries, and run settlement.
It stores opaque bodies and bounded envelopes in PostgreSQL and supplies the
dispatch port consumed by `mailbox-dispatch`.

# Responsibilities

`accept` validates body length and integrity, inserts payload and event data in
one transaction, and deduplicates by mailbox and producer key. The database
creates the initial delivery and identifier-only wake command in that commit.
Dispatch rechecks current mailbox, instance, revision, release, attachment,
and run-gate state under an instance advisory lock before leasing an attempt and
creating its run. Operator actions are authorized and redacted, while retry,
cancel, dead-letter, stale-claim recovery, and run completion update durable
state without exposing opaque bodies.

# When

Create the repository over the control-plane pool and pass it to acceptance,
dispatch, and operator workflows:

```rust
let repository = PostgresMailboxRepository::new(pool);
let accepted = repository.accept(project_id, &event, encoded_body, decoded_length).await?;
```

Use the returned event identity and duplicate flag to report acceptance; let
the outbox publisher wake the dispatcher after the transaction commits.
