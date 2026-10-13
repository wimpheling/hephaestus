//! Actual `PostgreSQL` read-only observation retirement; no provider proof fixtures.
#[path = "retirement/behavior.rs"]
mod behavior;
#[path = "retirement/concurrency.rs"]
mod concurrency;
#[path = "support/fixtures.rs"]
#[allow(dead_code)] // Shared admission declaration helpers include unrelated cases.
mod fixtures;
#[path = "retirement/guards.rs"]
mod guards;
#[path = "execution/harness.rs"]
#[allow(dead_code)] // Reuse actual admission/claim fixtures without provider completion.
mod harness;
#[path = "retirement/helpers.rs"]
mod helpers;
#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
#[allow(dead_code)] // Shared historical seed contains other authorization cases.
mod seed;
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Reuse fixture identities; this target supplies role pools.
mod support;
#[path = "retirement/upgrade.rs"]
mod upgrade;

use fixtures::private_migrations;
