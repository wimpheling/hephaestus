//! Opt-in real `PostgreSQL` and `JetStream` proof for durable mailbox acceptance.

#[path = "postgres/acceptance.rs"]
mod acceptance;
#[path = "postgres/acceptance_recovery.rs"]
mod acceptance_recovery;
#[path = "postgres/acceptance_setup.rs"]
mod acceptance_setup;
#[path = "postgres/acceptance_transport.rs"]
mod acceptance_transport;
#[path = "postgres/allocation.rs"]
mod allocation;
#[path = "postgres/empty.rs"]
mod empty;
#[path = "postgres/recovery.rs"]
mod recovery;
#[path = "postgres/retention.rs"]
mod retention;
#[path = "postgres/rls.rs"]
mod rls;
#[path = "postgres/support.rs"]
mod support;
