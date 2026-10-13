//! Actual application-engine ledger, native owned disk and typed import admission.
//! No guest execution, runtime invocation, startup or completed `SQLite` claim.

#[path = "owned_first_ready/fixture.rs"]
#[allow(dead_code)] // Genuine disk/root and completed-custody setup, registered once.
mod birth;
#[path = "recipe_engine/cancellation.rs"]
mod cancellation;
#[path = "recipe_engine/fixture.rs"]
mod fixture;
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Existing genuine immutable intent helpers and role pools.
mod fixtures;
#[path = "recipe_engine/install.rs"]
mod install;
#[path = "recipe_instance_import/fixture.rs"]
#[allow(dead_code)] // Shared published Source and genuine original Install harness.
mod instance_fixture;
#[path = "recipe_engine/recovery.rs"]
mod recovery;
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod schema;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Same exact published workload contract and server policy.
mod source_fixture;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Real seeded users; no recorded-actor authentication conversion.
mod support;
#[path = "prepared_owned_volume/fixture.rs"]
#[allow(dead_code)] // Already checked owning production driver and native helpers.
mod volume_fixture;
