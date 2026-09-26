//! Opt-in `PostgreSQL` integration coverage for durable run persistence.

#[path = "postgres/idempotency.rs"]
mod idempotency;
#[path = "postgres/retry_catalog.rs"]
mod retry_catalog;
#[path = "postgres/runtime_git.rs"]
mod runtime_git;
#[path = "postgres/support.rs"]
mod support;
