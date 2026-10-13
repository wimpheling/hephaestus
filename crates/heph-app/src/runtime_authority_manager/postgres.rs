//! Actual manager/session/handoff checks; trusted fixtures are not HTTP proof.

#[path = "../../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
// Reuse the actual actor graph; unrelated original authz entry points are unused.
#[allow(dead_code)]
mod seed;
#[path = "../../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
// Reuse the actual actor graph; unrelated original authz entry points are unused.
#[allow(dead_code)]
mod support;

#[path = "postgres_denials.rs"]
mod denials;
#[path = "postgres_fixture.rs"]
pub mod fixture;
#[path = "postgres_phases.rs"]
mod phases;
