//! Real `PostgreSQL` and bare-Git verification for durable review controls.

#[path = "postgres_git/admission.rs"]
mod admission;
#[path = "postgres_git/conflict.rs"]
mod conflict;
#[path = "postgres_git/controls.rs"]
mod controls;
#[path = "postgres_git/rejection.rs"]
mod rejection;
#[path = "postgres_git/support/mod.rs"]
mod support;
