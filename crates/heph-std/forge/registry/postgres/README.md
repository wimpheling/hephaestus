# Purpose

`registry-postgres` persists the registry control-plane state behind Forge's
publication contracts. It stores publication ownership and lifecycle,
notification inbox rows and claims, digest and evidence projections, and the
operational data used by reconciliation workers.

# Responsibilities

The adapter maps validated registry-domain values to PostgreSQL rows and keeps
claim, transition, completion, retry, and failure changes transactional. It
uses the authorization boundary for tenant and administrator decisions and
records enough immutable digest and policy evidence for later reads. Callback
deduplication happens at the inbox boundary; a callback alone never approves
content. SQL and row conversion stay in this declared PostgreSQL adapter so
callers receive domain values instead of database records.

# When

Create the store during daemon composition and give it to registry workers and
notification ingestion:

```rust
let store = registry_postgres::connect(database_url).await?;
```

Workers use the store to claim notifications or publication work, reconcile
with Zot, and commit the resulting state before reporting success.
