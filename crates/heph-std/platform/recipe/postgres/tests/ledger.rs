//! Real adapter tests; invoke explicitly with disposable `DATABASE_URL`.

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Shared historical fixture includes unrelated authorization cases.
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse fixture identities; this target supplies its own app pool.
mod support;

#[path = "support/admission.rs"]
mod admission;
#[path = "support/authorization.rs"]
mod authorization;
#[path = "support/fixtures.rs"]
mod fixtures;
#[path = "support/guards.rs"]
mod guards;
#[path = "support/upgrade.rs"]
mod upgrade;
