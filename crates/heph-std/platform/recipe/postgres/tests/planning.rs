//! Actual authoritative planner tests, requiring disposable `DATABASE_URL`.

#[path = "support/fixtures.rs"]
#[allow(dead_code)] // Reuse the established app-role pool and recipe declarations.
mod fixtures;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Shared fixture contains unrelated authorization cases.
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Shared identities use the verified actor transaction path.
mod support;

#[path = "planning/authorization.rs"]
mod authorization;
#[path = "planning/external.rs"]
mod external;
#[path = "planning/fixture.rs"]
mod fixture;
#[path = "planning/sources.rs"]
mod sources;
