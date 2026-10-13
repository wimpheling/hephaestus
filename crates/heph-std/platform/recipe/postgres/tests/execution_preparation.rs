//! Actual private113 preparation tests; metadata only, no provider/import IO.

#[path = "support/fixtures.rs"]
#[allow(dead_code)] // Established verified actor pool and declaration helpers.
mod fixtures;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Established valid cross-table authorization graph.
mod seed;
#[path = "planning/fixture.rs"]
#[allow(dead_code)] // Published authored source fixture shared with planning.
mod source_fixture;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Verified identities; unrelated shared helpers remain unused.
mod support;

#[path = "execution_preparation/authorization.rs"]
mod authorization;
#[path = "execution_preparation/guards.rs"]
mod guards;
#[path = "execution_preparation/harness.rs"]
mod harness;
#[path = "execution_preparation/history.rs"]
mod history;
#[path = "execution_preparation/records.rs"]
mod records;
#[path = "execution_preparation/schema.rs"]
mod schema;
