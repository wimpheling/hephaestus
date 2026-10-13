//! Actual closed Ready retention and immutable Remove history over PostgreSQL/FS.
//! Controlled VM scope pins are not physical VM ownership or destruction proof.

#[path = "owned_first_ready/fixture.rs"]
#[allow(dead_code)] // Reuse genuine original creation/custody and whole preparation.
mod birth;
#[path = "owned_ready_retention/cancellation.rs"]
mod cancellation;
#[path = "owned_ready_retention/fixture.rs"]
mod fixture;
#[path = "../../heph-std/platform/recipe/postgres/tests/support/fixtures.rs"]
#[allow(dead_code)] // Genuine parsed admissions; no counterfeit actor identity.
mod fixtures;
#[path = "recipe_instance_import/fixture.rs"]
#[allow(dead_code)] // Existing published release/configuration fixture.
mod instance_fixture;
#[path = "owned_ready_retention/management.rs"]
mod management;
#[path = "owned_ready_retention/physical.rs"]
mod physical;
#[path = "owned_ready_retention/retention.rs"]
mod retention;
#[path = "owned_ready_retention/reuse.rs"]
mod reuse;
#[path = "../../heph-std/platform/recipe/postgres/tests/execution_preparation/schema.rs"]
mod schema;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
#[path = "../../heph-std/platform/recipe/postgres/tests/planning/fixture.rs"]
#[allow(dead_code)] // Existing genuine immutable catalog/whole platform policy.
mod source_fixture;
#[path = "../../heph-std/authorization/authz-postgres/tests/postgres/support.rs"]
#[allow(dead_code)] // Genuine verified test principals reused without reconstruction.
mod support;
